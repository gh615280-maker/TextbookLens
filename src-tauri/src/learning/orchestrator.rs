use std::{fmt, future::Future, pin::Pin, sync::Arc};

use futures_util::StreamExt;
use sha2::{Digest, Sha256};
use sqlx::SqlitePool;
use uuid::Uuid;

use crate::{
    ai::{
        provider::ProviderStream,
        runtime::{LoadedProvider, ProviderRuntime},
    },
    db::{
        conversations::LearningCommitAuthorizer,
        conversations::{
            BookFollowupCompletion, FollowupCompletion, LearningRepository,
            NewBookQuestionCompletion, NewSelectionCompletion,
        },
        messages::CompletedAssistantMessage,
    },
    domain::{
        AiOperation, ContentAnchor, ImageMime, LearningAction, LearningRequestSnapshot,
        UnifiedChatRequest, UnifiedStreamEvent, UnifiedVisionRequest, VisionAsset, VisionAssetMeta,
    },
    errors::{AppError, AppErrorCode, AppResult},
    maintenance::gate::NormalOperationPermit,
};

use super::{
    book_preparation::PreparedBookLearningRequest,
    history::PreparedFollowupExecution,
    preparation::{LearningAction as PreparationAction, PreparedLearningRequest},
    registry::{
        BookFollowupRequestContext, LearningPersistenceTarget, LearningRequestContext,
        LearningRequestRegistry, NewBookQuestionRequestContext, NewSelectionRequestContext,
    },
};

type BoxLearningTask = Pin<Box<dyn Future<Output = ()> + Send + 'static>>;

pub(crate) trait LearningTaskSpawner: Send + Sync {
    fn spawn(&self, task: BoxLearningTask) -> Result<(), ()>;
}

struct TokioLearningTaskSpawner;

impl LearningTaskSpawner for TokioLearningTaskSpawner {
    fn spawn(&self, task: BoxLearningTask) -> Result<(), ()> {
        let handle = tokio::runtime::Handle::try_current().map_err(|_| ())?;
        handle.spawn(task);
        Ok(())
    }
}

enum LearningProviderRequest {
    Text(UnifiedChatRequest),
    Vision(UnifiedVisionRequest),
}

impl fmt::Debug for LearningProviderRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::Text(_) => "LearningProviderRequest::Text(<redacted>)",
            Self::Vision(_) => "LearningProviderRequest::Vision(<redacted>)",
        })
    }
}

#[derive(Clone)]
pub struct LearningOrchestrator {
    registry: LearningRequestRegistry,
    pool: SqlitePool,
    repository: LearningRepository,
}

impl LearningOrchestrator {
    pub fn new(registry: LearningRequestRegistry, pool: SqlitePool) -> Self {
        Self {
            registry,
            repository: LearningRepository::new(pool.clone()),
            pool,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_repository(
        registry: LearningRequestRegistry,
        pool: SqlitePool,
        repository: LearningRepository,
    ) -> Self {
        Self {
            registry,
            pool,
            repository,
        }
    }

    pub async fn start_prepared(
        &self,
        runtime: &ProviderRuntime,
        prepared: PreparedLearningRequest,
    ) -> AppResult<LearningRequestSnapshot> {
        let maintenance_permit = self.registry.acquire_maintenance_permit()?;
        self.start_prepared_with_maintenance_permit(runtime, prepared, maintenance_permit)
            .await
    }

    pub(crate) async fn start_prepared_with_maintenance_permit(
        &self,
        runtime: &ProviderRuntime,
        prepared: PreparedLearningRequest,
        maintenance_permit: NormalOperationPermit,
    ) -> AppResult<LearningRequestSnapshot> {
        let operation = if prepared.requires_vision() {
            AiOperation::VisionLearning
        } else {
            AiOperation::TextLearning
        };
        let loaded = runtime
            .load(&self.pool, prepared.provider_profile_id(), operation)
            .await?;
        if loaded.profile().id != prepared.provider_profile_id()
            || loaded.profile().model_id != prepared.model_id()
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }

        let question = prepared.current_question()?.to_owned();
        let model_id = prepared.model_id().to_owned();
        let chat = prepared
            .prepared_prompt()
            .clone()
            .into_chat_request(model_id.clone(), prepared.default_max_output_tokens());
        let request = if prepared.requires_vision() {
            let capture = prepared
                .capture()
                .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
            let bytes = capture.bytes().to_vec();
            let sha256 = Sha256::digest(&bytes)
                .iter()
                .map(|byte| format!("{byte:02x}"))
                .collect();
            let meta = VisionAssetMeta {
                book_id: prepared.book_id(),
                capture_id: Uuid::new_v4(),
                mime_type: ImageMime::Png,
                width: capture.width(),
                height: capture.height(),
                encoded_bytes: u64::try_from(bytes.len())
                    .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?,
                sha256,
            };
            LearningProviderRequest::Vision(UnifiedVisionRequest {
                text: chat,
                images: vec![VisionAsset::from_validated(meta, bytes)],
            })
        } else {
            LearningProviderRequest::Text(chat)
        };
        let selected_text = selected_text_for_persistence(prepared.anchor());
        let context = Arc::new(LearningRequestContext {
            target: LearningPersistenceTarget::NewSelection(NewSelectionRequestContext {
                book_id: prepared.book_id(),
                section_id: prepared.section_id(),
                anchor: prepared.anchor().clone(),
                selected_text,
                action: map_action(prepared.action()),
                question,
            }),
            provider_profile_id: prepared.provider_profile_id(),
            model_id,
            available_citations: prepared.packed_context().citations.clone(),
        });
        self.start_loaded(
            loaded,
            request,
            context,
            maintenance_permit,
            &TokioLearningTaskSpawner,
        )
    }

    pub async fn start_prepared_book(
        &self,
        runtime: &ProviderRuntime,
        prepared: PreparedBookLearningRequest,
    ) -> AppResult<LearningRequestSnapshot> {
        let maintenance_permit = self.registry.acquire_maintenance_permit()?;
        self.start_prepared_book_with_maintenance_permit(runtime, prepared, maintenance_permit)
            .await
    }

    pub(crate) async fn start_prepared_book_with_maintenance_permit(
        &self,
        runtime: &ProviderRuntime,
        prepared: PreparedBookLearningRequest,
        maintenance_permit: NormalOperationPermit,
    ) -> AppResult<LearningRequestSnapshot> {
        let question = prepared.current_question()?.to_owned();
        let book_id = prepared.book_id();
        let target = match (prepared.conversation_id(), prepared.expected_next_ordinal()) {
            (None, None) => {
                LearningPersistenceTarget::NewBookQuestion(NewBookQuestionRequestContext {
                    book_id,
                    question,
                })
            }
            (Some(conversation_id), Some(expected_next_ordinal)) => {
                LearningPersistenceTarget::BookFollowup(BookFollowupRequestContext {
                    book_id,
                    conversation_id,
                    expected_next_ordinal,
                    question,
                })
            }
            _ => return Err(AppError::new(AppErrorCode::RequestConflict)),
        };
        let profile_id = prepared.provider_profile_id();
        let provider_kind = prepared.provider_kind().clone();
        let model_id = prepared.model_id().to_owned();
        let request = LearningProviderRequest::Text(prepared.chat_request());
        let context = Arc::new(LearningRequestContext {
            target,
            provider_profile_id: profile_id,
            model_id: model_id.clone(),
            available_citations: prepared.packed_context().citations.clone(),
        });
        let snapshot = self
            .registry
            .create_with_maintenance_permit(context, maintenance_permit)?;
        let request_id = snapshot.request_id;
        let _provider_load_lease = self.registry.operation_permit(request_id)?;

        let loaded = match runtime
            .load(&self.pool, profile_id, AiOperation::TextLearning)
            .await
        {
            Ok(loaded) => loaded,
            Err(error) => return self.fail_registered_start(request_id, &error),
        };
        if loaded.profile().id != profile_id
            || loaded.profile().kind != provider_kind
            || loaded.provider_kind() != provider_kind
            || loaded.profile().model_id != model_id
        {
            return self
                .fail_registered_start(request_id, &AppError::new(AppErrorCode::RequestConflict));
        }
        if self.registry.cancellation_token(request_id)?.is_cancelled() {
            self.registry.release_maintenance_permit(request_id);
            return self.registry.snapshot(request_id);
        }
        self.start_registered_loaded(snapshot, loaded, request, &TokioLearningTaskSpawner)
    }

    pub async fn start_followup(
        &self,
        runtime: &ProviderRuntime,
        prepared: PreparedFollowupExecution,
    ) -> AppResult<LearningRequestSnapshot> {
        let maintenance_permit = self.registry.acquire_maintenance_permit()?;
        self.start_followup_with_maintenance_permit(runtime, prepared, maintenance_permit)
            .await
    }

    pub(crate) async fn start_followup_with_maintenance_permit(
        &self,
        runtime: &ProviderRuntime,
        prepared: PreparedFollowupExecution,
        maintenance_permit: NormalOperationPermit,
    ) -> AppResult<LearningRequestSnapshot> {
        let loaded = runtime
            .load(
                &self.pool,
                prepared.context.provider_profile_id,
                prepared.operation,
            )
            .await?;
        if loaded.profile().id != prepared.context.provider_profile_id
            || loaded.profile().model_id != prepared.context.model_id
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        self.start_loaded(
            loaded,
            LearningProviderRequest::Text(prepared.chat_request),
            prepared.context,
            maintenance_permit,
            &TokioLearningTaskSpawner,
        )
    }

    fn start_loaded(
        &self,
        loaded: LoadedProvider,
        request: LearningProviderRequest,
        context: Arc<LearningRequestContext>,
        maintenance_permit: NormalOperationPermit,
        spawner: &dyn LearningTaskSpawner,
    ) -> AppResult<LearningRequestSnapshot> {
        let snapshot = self
            .registry
            .create_with_maintenance_permit(context, maintenance_permit)?;
        self.start_registered_loaded(snapshot, loaded, request, spawner)
    }

    fn start_registered_loaded(
        &self,
        snapshot: LearningRequestSnapshot,
        loaded: LoadedProvider,
        request: LearningProviderRequest,
        spawner: &dyn LearningTaskSpawner,
    ) -> AppResult<LearningRequestSnapshot> {
        let request_id = snapshot.request_id;
        let operation_lease = self.registry.operation_permit(request_id)?;
        let orchestrator = self.clone();
        let maintenance_guard = LearningTaskMaintenanceGuard {
            registry: orchestrator.registry.clone(),
            request_id,
        };
        let task = Box::pin(async move {
            let _maintenance_guard = maintenance_guard;
            let _operation_lease = operation_lease;
            orchestrator
                .run_loaded_provider(request_id, loaded, request)
                .await;
        });
        self.spawn_registered(snapshot, task, spawner)
    }

    fn fail_registered_start(
        &self,
        request_id: Uuid,
        error: &AppError,
    ) -> AppResult<LearningRequestSnapshot> {
        self.registry.fail(request_id, error)?;
        self.registry.release_maintenance_permit(request_id);
        self.registry.snapshot(request_id)
    }

    fn spawn_registered(
        &self,
        snapshot: LearningRequestSnapshot,
        task: BoxLearningTask,
        spawner: &dyn LearningTaskSpawner,
    ) -> AppResult<LearningRequestSnapshot> {
        let request_id = snapshot.request_id;
        if spawner.spawn(task).is_err() {
            let error = AppError::new(AppErrorCode::ProviderUnavailable);
            self.registry.fail(request_id, &error)?;
            return self.registry.snapshot(request_id);
        }
        Ok(snapshot)
    }

    async fn run_loaded_provider(
        &self,
        request_id: Uuid,
        loaded: LoadedProvider,
        request: LearningProviderRequest,
    ) {
        let cancel = match self.registry.cancellation_token(request_id) {
            Ok(cancel) => cancel,
            Err(_) => return,
        };
        let stream = match request {
            LearningProviderRequest::Text(request) => {
                loaded.stream_text_learning(request, cancel.clone()).await
            }
            LearningProviderRequest::Vision(request) => {
                loaded.stream_vision_learning(request, cancel.clone()).await
            }
        };
        match stream {
            Ok(stream) => self.run_stream(request_id, stream).await,
            Err(error) => {
                let _ = self.registry.fail(request_id, &error);
            }
        }
    }

    #[doc(hidden)]
    pub async fn run_stream(&self, request_id: Uuid, mut stream: ProviderStream) {
        while let Some(event) = stream.next().await {
            let event = match event {
                Ok(event) => event,
                Err(error) => {
                    let _ = self.registry.fail(request_id, &error.into_app_error());
                    return;
                }
            };
            let result = match event {
                UnifiedStreamEvent::TextDelta { text } => {
                    self.registry.append_text(request_id, text)
                }
                UnifiedStreamEvent::Usage {
                    input_tokens,
                    output_tokens,
                } => self
                    .registry
                    .record_usage(request_id, input_tokens, output_tokens),
                UnifiedStreamEvent::Completed => {
                    self.commit_completed(request_id).await;
                    return;
                }
            };
            if let Err(error) = result {
                let _ = self.registry.fail(request_id, &error);
                return;
            }
        }
        let error = AppError::new(AppErrorCode::ProviderUnavailable);
        let _ = self.registry.fail(request_id, &error);
    }

    async fn commit_completed(&self, request_id: Uuid) {
        let should_commit = match self.registry.begin_commit(request_id) {
            Ok(value) => value,
            Err(error) => {
                let _ = self.registry.fail(request_id, &error);
                return;
            }
        };
        if !should_commit {
            return;
        }
        let context = match self.registry.commit_context(request_id) {
            Ok(context) => context,
            Err(error) => {
                let _ = self.registry.fail(request_id, &error);
                return;
            }
        };
        let answer = match self.registry.snapshot(request_id) {
            Ok(snapshot) => snapshot.text,
            Err(error) => {
                let _ = self.registry.fail(request_id, &error);
                return;
            }
        };
        let assistant = CompletedAssistantMessage {
            provider_profile_id: context.provider_profile_id,
            model_id: context.model_id.clone(),
            answer,
            available_citations: context.available_citations.clone(),
        };
        let commit_authorizer = RegistryCommitAuthorizer {
            registry: self.registry.clone(),
            request_id,
        };
        let persisted = match &context.target {
            LearningPersistenceTarget::NewSelection(target) => self
                .repository
                .persist_new_selection_authorized(
                    NewSelectionCompletion {
                        book_id: target.book_id,
                        section_id: target.section_id,
                        anchor: target.anchor.clone(),
                        selected_text: target.selected_text.clone(),
                        action: target.action.clone(),
                        question: target.question.clone(),
                        assistant,
                    },
                    &commit_authorizer,
                )
                .await
                .map(|result| result.conversation_id),
            LearningPersistenceTarget::SelectionFollowup(target) => self
                .repository
                .persist_followup_authorized(
                    FollowupCompletion {
                        book_id: target.book_id,
                        conversation_id: target.conversation_id,
                        expected_next_ordinal: target.expected_next_ordinal,
                        question: target.question.clone(),
                        assistant,
                    },
                    &commit_authorizer,
                )
                .await
                .map(|()| target.conversation_id),
            LearningPersistenceTarget::NewBookQuestion(target) => self
                .repository
                .persist_new_book_question_authorized(
                    NewBookQuestionCompletion {
                        book_id: target.book_id,
                        question: target.question.clone(),
                        assistant,
                    },
                    &commit_authorizer,
                )
                .await
                .map(|result| result.conversation_id),
            LearningPersistenceTarget::BookFollowup(target) => self
                .repository
                .persist_book_followup_authorized(
                    BookFollowupCompletion {
                        book_id: target.book_id,
                        conversation_id: target.conversation_id,
                        expected_next_ordinal: target.expected_next_ordinal,
                        question: target.question.clone(),
                        assistant,
                    },
                    &commit_authorizer,
                )
                .await
                .map(|()| target.conversation_id),
        };
        match persisted {
            Ok(conversation_id) => {
                let _ = self.registry.complete(request_id, conversation_id);
            }
            Err(error) => {
                let _ = self.registry.fail(request_id, &error);
            }
        }
    }
}

struct LearningTaskMaintenanceGuard {
    registry: LearningRequestRegistry,
    request_id: Uuid,
}

impl Drop for LearningTaskMaintenanceGuard {
    fn drop(&mut self) {
        self.registry.release_maintenance_permit(self.request_id);
    }
}

struct RegistryCommitAuthorizer {
    registry: LearningRequestRegistry,
    request_id: Uuid,
}

impl LearningCommitAuthorizer for RegistryCommitAuthorizer {
    fn authorize_commit(&self) -> AppResult<()> {
        if self.registry.authorize_database_commit(self.request_id)? {
            Ok(())
        } else {
            Err(AppError::new(AppErrorCode::ImportCancelled))
        }
    }
}

impl fmt::Debug for LearningOrchestrator {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LearningOrchestrator")
            .field("registry", &self.registry)
            .field("repository", &self.repository)
            .finish()
    }
}

fn selected_text_for_persistence(anchor: &ContentAnchor) -> Option<String> {
    match anchor {
        ContentAnchor::Text { selection } => Some(selection.quote.exact.clone()),
        ContentAnchor::Region { region } => region
            .text_fallback
            .as_ref()
            .map(|fallback| fallback.exact.clone()),
    }
}

fn map_action(action: PreparationAction) -> LearningAction {
    match action {
        PreparationAction::Explain => LearningAction::Explain,
        PreparationAction::Example => LearningAction::Example,
        PreparationAction::Derive => LearningAction::Derive,
        PreparationAction::Translate => LearningAction::Translate,
        PreparationAction::Ask => LearningAction::Ask,
    }
}

#[cfg(test)]
#[path = "orchestrator_test.rs"]
mod tests;
