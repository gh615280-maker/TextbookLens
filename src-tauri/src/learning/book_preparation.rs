use std::{
    collections::BTreeMap,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use chrono::{DateTime, SecondsFormat, Utc};
use parking_lot::Mutex;
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool, sqlite::SqliteRow};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    ai::registry::ProviderCapabilityRegistry,
    credentials::CredentialStore,
    db::{messages::validate_question, providers},
    domain::{
        ActiveOperationKind, AiOperation, BookFormat, BookLearningPreparationRiskFlag,
        BookLearningPreparationSummary, CapabilitySupport, Citation, CitationReviewStatus,
        ContextMode, ConversationScope, DocumentLocator, PrepareBookLearningRequestMetadata,
        ProviderKind, ProviderOperationConsent, ProviderOperationConsentCategory,
        ProviderOperationConsentDecision, TeachingInstructionDto, UiLanguage, UnifiedChatRequest,
        UnifiedMessage, UnifiedRole,
    },
    errors::{AppError, AppErrorCode, AppResult},
    maintenance::gate::MaintenanceGate,
    retrieval::{
        budget::{InputBudget, STANDARD_CONTEXT_CAP, conservative_token_count},
        context::{
            ContextCandidate, ContextSourceKind, PackedContext, book_candidate_from_search_hit,
        },
        search::search_book_relevant_in_transaction,
    },
};

use super::{
    ContextSource, PreparedPrompt, PromptInput, PromptOperation, PromptPolicy,
    preparation::{
        InvalidateLearningPreparations, LearningAuthorizationDecision, LearningInvalidationReason,
    },
};

pub const BOOK_PREPARATION_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_BOOK_PREPARATIONS: usize = 64;
const MAX_TEXT_BYTES_PER_PREPARATION: usize = 2 * 1024 * 1024;
const MAX_TOTAL_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_OUTLINE_ITEMS: usize = 256;
const MAX_OUTLINE_TOTAL_BYTES: usize = 64 * 1024;
const MAX_OUTLINE_TOKENS: u64 = 16 * 1024;
const MAX_SECTION_TITLE_BYTES: usize = 16 * 1024;
const MAX_SECTION_TITLE_CODE_POINTS: usize = 4_096;
const MAX_SECTION_LOCATOR_JSON_BYTES: usize = 16 * 1024;
const MAX_RETRIEVAL_RESULTS: usize = 24;
const MAX_RETRIEVAL_FETCH: u32 = 64;
const MAX_RETRIEVAL_ITEM_BYTES: usize = 64 * 1024;
const MAX_RETRIEVAL_TOTAL_BYTES: usize = 256 * 1024;
const MAX_RETRIEVAL_TOKENS: u64 = 64 * 1024;
const MAX_HISTORY_MESSAGES: usize = 32;
const MAX_HISTORY_TOTAL_BYTES: usize = 256 * 1024;
const MAX_HISTORY_TOKENS: u64 = 64 * 1024;
const MAX_HISTORY_CITATIONS_JSON_BYTES: usize = 1024 * 1024;
const HISTORY_MESSAGE_FRAMING_TOKENS: u64 = 16;
const MAX_MODEL_ID_CODE_POINTS: usize = 256;
const MAX_DISPLAY_NAME_BYTES: usize = 2 * 1024;
const MAX_DISPLAY_NAME_CODE_POINTS: usize = 512;

#[derive(Clone, PartialEq, Eq)]
struct BookBindingSnapshot {
    book_id: Uuid,
    format: BookFormat,
    source_sha256: String,
    updated_at: String,
}

impl fmt::Debug for BookBindingSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BookBindingSnapshot")
            .field("book_id", &"<redacted>")
            .field("format", &self.format)
            .field("source_sha256", &"<redacted>")
            .field("updated_at", &"<redacted>")
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
struct ProviderBindingSnapshot {
    book: BookBindingSnapshot,
    profile_id: Uuid,
    provider_kind: ProviderKind,
    provider_display_name: String,
    profile_display_name: String,
    model_id: String,
    model_display_name: String,
    context_window_tokens: u32,
    default_max_output_tokens: u32,
    validated_at: DateTime<Utc>,
    profile_updated_at: String,
    context_mode: ContextMode,
    ui_language: UiLanguage,
}

impl fmt::Debug for ProviderBindingSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderBindingSnapshot")
            .field("book", &self.book)
            .field("profile_id", &"<redacted>")
            .field("provider_kind", &self.provider_kind)
            .field("provider_display_name", &self.provider_display_name)
            .field("profile_display_name", &self.profile_display_name)
            .field("model_id", &"<redacted>")
            .field("model_display_name", &self.model_display_name)
            .field("context_window_tokens", &self.context_window_tokens)
            .field("default_max_output_tokens", &self.default_max_output_tokens)
            .field("validated_at", &"<redacted>")
            .field("profile_updated_at", &"<redacted>")
            .field("context_mode", &self.context_mode)
            .field("ui_language", &self.ui_language)
            .finish()
    }
}

#[derive(Clone, PartialEq, Eq)]
enum BookPreparationTarget {
    New,
    Continue {
        conversation_id: Uuid,
        expected_next_ordinal: u32,
    },
}

impl fmt::Debug for BookPreparationTarget {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::New => formatter.write_str("New"),
            Self::Continue {
                expected_next_ordinal,
                ..
            } => formatter
                .debug_struct("Continue")
                .field("conversation_id", &"<redacted>")
                .field("expected_next_ordinal", expected_next_ordinal)
                .finish(),
        }
    }
}

#[derive(Clone, PartialEq, Eq)]
struct SnapshotFingerprint([u8; 32]);

impl fmt::Debug for SnapshotFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted-snapshot-fingerprint>")
    }
}

impl Drop for SnapshotFingerprint {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

#[derive(Clone, PartialEq, Eq)]
struct ConsentFingerprint([u8; 32]);

impl fmt::Debug for ConsentFingerprint {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("<redacted-consent-fingerprint>")
    }
}

impl Drop for ConsentFingerprint {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

struct ConsentSnapshot {
    fingerprint: ConsentFingerprint,
    cost_risk: ProviderOperationConsentDecision,
}

struct LocalPreparationSnapshot {
    binding: ProviderBindingSnapshot,
    teaching_instruction: TeachingInstructionDto,
    target: BookPreparationTarget,
    outline: Vec<ContextCandidate>,
    retrieval: Vec<ContextCandidate>,
    history: Vec<UnifiedMessage>,
    total_history_messages: u32,
    omitted_outline_items: u32,
    omitted_retrieval_items: u32,
    history_was_omitted: bool,
    fingerprint: SnapshotFingerprint,
}

impl fmt::Debug for LocalPreparationSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LocalPreparationSnapshot")
            .field("binding", &self.binding)
            .field("teaching_instruction", &"<redacted>")
            .field("target", &self.target)
            .field("outline_count", &self.outline.len())
            .field("retrieval_count", &self.retrieval.len())
            .field("history_message_count", &self.history.len())
            .field("total_history_messages", &self.total_history_messages)
            .field("omitted_outline_items", &self.omitted_outline_items)
            .field("omitted_retrieval_items", &self.omitted_retrieval_items)
            .field("history_was_omitted", &self.history_was_omitted)
            .field("fingerprint", &self.fingerprint)
            .finish()
    }
}

impl Drop for LocalPreparationSnapshot {
    fn drop(&mut self) {
        self.binding.source_strings_zeroize();
        self.teaching_instruction.instruction.zeroize();
        for candidate in self.outline.iter_mut().chain(self.retrieval.iter_mut()) {
            zeroize_candidate(candidate);
        }
        for message in &mut self.history {
            message.content.zeroize();
        }
    }
}

pub struct PreparedBookLearningRequest {
    binding: ProviderBindingSnapshot,
    target: BookPreparationTarget,
    teaching_instruction: TeachingInstructionDto,
    prepared_prompt: PreparedPrompt,
    packed_context: PackedContext,
    snapshot_fingerprint: SnapshotFingerprint,
    consent_fingerprint: ConsentFingerprint,
    risk_flags: Vec<BookLearningPreparationRiskFlag>,
    authorization_required: bool,
}

impl PreparedBookLearningRequest {
    pub fn provider_profile_id(&self) -> Uuid {
        self.binding.profile_id
    }

    pub fn provider_kind(&self) -> &ProviderKind {
        &self.binding.provider_kind
    }

    pub fn model_id(&self) -> &str {
        &self.binding.model_id
    }

    pub fn book_id(&self) -> Uuid {
        self.binding.book.book_id
    }

    pub const fn conversation_scope(&self) -> ConversationScope {
        ConversationScope::Book
    }

    pub fn conversation_id(&self) -> Option<Uuid> {
        match self.target {
            BookPreparationTarget::New => None,
            BookPreparationTarget::Continue {
                conversation_id, ..
            } => Some(conversation_id),
        }
    }

    pub fn expected_next_ordinal(&self) -> Option<u32> {
        match self.target {
            BookPreparationTarget::New => None,
            BookPreparationTarget::Continue {
                expected_next_ordinal,
                ..
            } => Some(expected_next_ordinal),
        }
    }

    pub fn default_max_output_tokens(&self) -> u32 {
        self.binding.default_max_output_tokens
    }

    pub fn teaching_instruction(&self) -> &TeachingInstructionDto {
        &self.teaching_instruction
    }

    pub fn prepared_prompt(&self) -> &PreparedPrompt {
        &self.prepared_prompt
    }

    pub fn packed_context(&self) -> &PackedContext {
        &self.packed_context
    }

    pub fn current_question(&self) -> AppResult<&str> {
        self.prepared_prompt
            .messages
            .last()
            .filter(|message| message.role == UnifiedRole::User)
            .map(|message| message.content.as_str())
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))
    }

    /// Returns the exact immutable text request prepared here. It does not access a provider.
    pub fn chat_request(&self) -> UnifiedChatRequest {
        self.prepared_prompt.clone().into_chat_request(
            self.binding.model_id.clone(),
            self.binding.default_max_output_tokens,
        )
    }
}

impl fmt::Debug for PreparedBookLearningRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedBookLearningRequest")
            .field("binding", &self.binding)
            .field("target", &self.target)
            .field("teaching_instruction", &"<redacted>")
            .field("prepared_prompt", &self.prepared_prompt)
            .field("packed_context", &self.packed_context)
            .field("snapshot_fingerprint", &self.snapshot_fingerprint)
            .field("consent_fingerprint", &self.consent_fingerprint)
            .field("risk_flags", &self.risk_flags)
            .field("authorization_required", &self.authorization_required)
            .finish()
    }
}

impl Drop for PreparedBookLearningRequest {
    fn drop(&mut self) {
        self.binding.source_strings_zeroize();
        self.teaching_instruction.instruction.zeroize();
        self.prepared_prompt.system.zeroize();
        for message in &mut self.prepared_prompt.messages {
            message.content.zeroize();
        }
        for segment in &mut self.packed_context.segments {
            segment.content.zeroize();
            segment.locator_label.zeroize();
            segment.stable_id.zeroize();
            if let Some(seed) = &mut segment.citation_seed {
                seed.label.zeroize();
                zeroize_document_locator(&mut seed.locator);
            }
            if let Some(citation) = &mut segment.citation {
                zeroize_citation(citation);
            }
        }
        for citation in &mut self.packed_context.citations {
            zeroize_citation(citation);
        }
    }
}

impl ProviderBindingSnapshot {
    fn source_strings_zeroize(&mut self) {
        self.book.source_sha256.zeroize();
        self.book.updated_at.zeroize();
        self.provider_display_name.zeroize();
        self.profile_display_name.zeroize();
        self.model_id.zeroize();
        self.model_display_name.zeroize();
        self.profile_updated_at.zeroize();
    }
}

#[async_trait]
pub(crate) trait BookPreparationSensitiveAccess: Send + Sync {
    async fn require_credential(&self, profile_id: Uuid) -> AppResult<()>;

    async fn load_consents(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
    ) -> AppResult<Vec<ProviderOperationConsent>>;
}

struct RuntimeBookPreparationSensitiveAccess {
    credential_store: Arc<dyn CredentialStore>,
}

#[async_trait]
impl BookPreparationSensitiveAccess for RuntimeBookPreparationSensitiveAccess {
    async fn require_credential(&self, profile_id: Uuid) -> AppResult<()> {
        let credential = self
            .credential_store
            .get(&providers::credential_key(profile_id))
            .await?;
        drop(credential);
        Ok(())
    }

    async fn load_consents(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
    ) -> AppResult<Vec<ProviderOperationConsent>> {
        providers::list_provider_operation_consents(pool, profile_id).await
    }
}

#[derive(Clone)]
pub struct BookLearningPreparationService {
    pool: SqlitePool,
    registry: BookPreparationRegistry,
    capabilities: ProviderCapabilityRegistry,
    sensitive_access: Arc<dyn BookPreparationSensitiveAccess>,
    maintenance_gate: MaintenanceGate,
}

impl BookLearningPreparationService {
    pub fn new(
        pool: SqlitePool,
        registry: BookPreparationRegistry,
        capabilities: ProviderCapabilityRegistry,
        credential_store: Arc<dyn CredentialStore>,
        maintenance_gate: MaintenanceGate,
    ) -> Self {
        Self {
            pool,
            registry,
            capabilities,
            sensitive_access: Arc::new(RuntimeBookPreparationSensitiveAccess { credential_store }),
            maintenance_gate,
        }
    }

    #[cfg(test)]
    pub(crate) fn with_sensitive_access(
        mut self,
        sensitive_access: Arc<dyn BookPreparationSensitiveAccess>,
    ) -> Self {
        self.sensitive_access = sensitive_access;
        self
    }

    pub async fn prepare(
        &self,
        metadata: PrepareBookLearningRequestMetadata,
    ) -> AppResult<BookLearningPreparationSummary> {
        validate_question(metadata.question())?;
        let _permit = self
            .maintenance_gate
            .try_acquire_normal(ActiveOperationKind::Learning)
            .map_err(AppError::from)?;

        let snapshot = load_local_snapshot(&self.pool, &self.capabilities, &metadata).await?;
        let (prepared_prompt, packed_context, _retained_history_count) =
            pack_local_snapshot(&snapshot, metadata.question())?;
        if prepared_text_bytes(
            &snapshot.binding,
            &snapshot.teaching_instruction,
            &prepared_prompt,
            &packed_context,
        ) > MAX_TEXT_BYTES_PER_PREPARATION
        {
            return Err(AppError::new(AppErrorCode::ContextTooLarge));
        }

        // Credential, consent, and other provider-adjacent state are forbidden above this point.
        self.require_snapshot_unchanged(&metadata, &snapshot.fingerprint)
            .await?;
        self.sensitive_access
            .require_credential(snapshot.binding.profile_id)
            .await?;
        let consent = consent_snapshot(
            snapshot.binding.profile_id,
            self.sensitive_access
                .load_consents(&self.pool, snapshot.binding.profile_id)
                .await?,
        )?;
        self.require_snapshot_unchanged(&metadata, &snapshot.fingerprint)
            .await?;

        let mut risk_flags = Vec::new();
        if packed_context.estimated_input_tokens > STANDARD_CONTEXT_CAP {
            risk_flags.push(BookLearningPreparationRiskFlag::CostRisk);
        }
        let authorization_required = risk_flags
            .contains(&BookLearningPreparationRiskFlag::CostRisk)
            && consent.cost_risk == ProviderOperationConsentDecision::Ask;
        let source_count = u32::try_from(packed_context.segments.len())
            .map_err(|_| AppError::new(AppErrorCode::ContextTooLarge))?;
        let citation_count = u32::try_from(packed_context.citations.len())
            .map_err(|_| AppError::new(AppErrorCode::ContextTooLarge))?;
        let omitted_packed = packed_context.omitted_segment_count;
        let estimated_input_tokens = packed_context.estimated_input_tokens;
        self.registry.insert(
            PreparedBookLearningRequest {
                binding: snapshot.binding.clone(),
                target: snapshot.target.clone(),
                teaching_instruction: snapshot.teaching_instruction.clone(),
                prepared_prompt,
                packed_context,
                snapshot_fingerprint: snapshot.fingerprint.clone(),
                consent_fingerprint: consent.fingerprint,
                risk_flags: risk_flags.clone(),
                authorization_required,
            },
            BookSummaryDraft {
                provider_display_name: snapshot.binding.provider_display_name.clone(),
                profile_display_name: snapshot.binding.profile_display_name.clone(),
                model_display_name: snapshot.binding.model_display_name.clone(),
                estimated_input_tokens,
                source_count,
                citation_count,
                omitted_source_count: snapshot
                    .omitted_outline_items
                    .saturating_add(snapshot.omitted_retrieval_items)
                    .saturating_add(omitted_packed),
                risk_flags,
                requires_blocking_confirmation: authorization_required,
            },
        )
    }

    pub async fn authorize(
        &self,
        preparation_id: Uuid,
        decision: LearningAuthorizationDecision,
    ) -> AppResult<()> {
        if decision == LearningAuthorizationDecision::Deny {
            self.registry.discard(preparation_id);
            return Ok(());
        }
        let _permit = self
            .maintenance_gate
            .try_acquire_normal(ActiveOperationKind::Learning)
            .map_err(AppError::from)?;
        let binding = self.registry.binding(preparation_id, Instant::now())?;
        if !binding.authorization_required {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        self.revalidate_binding(&binding).await?;
        self.registry.authorize(preparation_id, Instant::now())
    }

    pub async fn consume_for_execution(
        &self,
        preparation_id: Uuid,
    ) -> AppResult<PreparedBookLearningRequest> {
        let permit = self
            .maintenance_gate
            .try_acquire_normal(ActiveOperationKind::Learning)
            .map_err(AppError::from)?;
        self.consume_for_execution_with_maintenance_permit(preparation_id, &permit)
            .await
    }

    pub(crate) async fn consume_for_execution_with_maintenance_permit(
        &self,
        preparation_id: Uuid,
        _maintenance_permit: &crate::maintenance::gate::NormalOperationPermit,
    ) -> AppResult<PreparedBookLearningRequest> {
        let binding = self.registry.binding(preparation_id, Instant::now())?;
        self.revalidate_binding(&binding).await?;
        self.registry.consume(preparation_id, Instant::now())
    }

    async fn require_snapshot_unchanged(
        &self,
        metadata: &PrepareBookLearningRequestMetadata,
        expected: &SnapshotFingerprint,
    ) -> AppResult<()> {
        let current = load_local_snapshot(&self.pool, &self.capabilities, metadata)
            .await
            .map_err(|_| AppError::new(AppErrorCode::RequestConflict))?;
        if &current.fingerprint != expected {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        Ok(())
    }

    async fn revalidate_binding(&self, binding: &BookRegistryBinding) -> AppResult<()> {
        let metadata = binding.metadata();
        if let Err(error) = self
            .require_snapshot_unchanged(&metadata, &binding.snapshot_fingerprint)
            .await
        {
            self.registry.discard(binding.preparation_id);
            return Err(error);
        }
        if let Err(error) = self
            .sensitive_access
            .require_credential(binding.profile_id)
            .await
        {
            self.registry.discard(binding.preparation_id);
            return Err(error);
        }
        let consents = match self
            .sensitive_access
            .load_consents(&self.pool, binding.profile_id)
            .await
        {
            Ok(consents) => consents,
            Err(error) => {
                self.registry.discard(binding.preparation_id);
                return Err(error);
            }
        };
        let consent = match consent_snapshot(binding.profile_id, consents) {
            Ok(consent) => consent,
            Err(error) => {
                self.registry.discard(binding.preparation_id);
                return Err(error);
            }
        };
        if consent.fingerprint != binding.consent_fingerprint {
            self.registry.discard(binding.preparation_id);
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        if let Err(error) = self
            .require_snapshot_unchanged(&metadata, &binding.snapshot_fingerprint)
            .await
        {
            self.registry.discard(binding.preparation_id);
            return Err(error);
        }
        Ok(())
    }
}

impl fmt::Debug for BookLearningPreparationService {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("BookLearningPreparationService(bounded-local-preparation)")
    }
}

#[derive(Clone)]
pub struct BookPreparationRegistry {
    inner: Arc<BookRegistryInner>,
}

struct BookRegistryInner {
    state: Mutex<BookRegistryState>,
}

impl Drop for BookRegistryInner {
    fn drop(&mut self) {
        self.state.get_mut().clear();
    }
}

#[derive(Default)]
struct BookRegistryState {
    entries: BTreeMap<Uuid, BookRegistryEntry>,
    total_text_bytes: usize,
}

impl BookRegistryState {
    fn clear(&mut self) {
        self.entries.clear();
        self.total_text_bytes = 0;
    }

    fn remove(&mut self, preparation_id: Uuid) -> Option<BookRegistryEntry> {
        let entry = self.entries.remove(&preparation_id)?;
        self.total_text_bytes = self.total_text_bytes.saturating_sub(entry.text_bytes);
        Some(entry)
    }
}

struct BookRegistryEntry {
    request: PreparedBookLearningRequest,
    text_bytes: usize,
    expires_at: Instant,
    authorized: bool,
}

struct BookSummaryDraft {
    provider_display_name: String,
    profile_display_name: String,
    model_display_name: String,
    estimated_input_tokens: u32,
    source_count: u32,
    citation_count: u32,
    omitted_source_count: u32,
    risk_flags: Vec<BookLearningPreparationRiskFlag>,
    requires_blocking_confirmation: bool,
}

#[derive(Clone)]
struct BookRegistryBinding {
    preparation_id: Uuid,
    book_id: Uuid,
    target: BookPreparationTarget,
    question: String,
    profile_id: Uuid,
    snapshot_fingerprint: SnapshotFingerprint,
    consent_fingerprint: ConsentFingerprint,
    authorization_required: bool,
}

impl BookRegistryBinding {
    fn metadata(&self) -> PrepareBookLearningRequestMetadata {
        match self.target {
            BookPreparationTarget::New => PrepareBookLearningRequestMetadata::New {
                book_id: self.book_id,
                question: self.question.clone(),
            },
            BookPreparationTarget::Continue {
                conversation_id, ..
            } => PrepareBookLearningRequestMetadata::Continue {
                book_id: self.book_id,
                conversation_id,
                question: self.question.clone(),
            },
        }
    }
}

impl fmt::Debug for BookRegistryBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BookRegistryBinding")
            .field("preparation_id", &"<redacted>")
            .field("book_id", &"<redacted>")
            .field("target", &self.target)
            .field("question_code_points", &self.question.chars().count())
            .field("profile_id", &"<redacted>")
            .field("snapshot_fingerprint", &self.snapshot_fingerprint)
            .field("consent_fingerprint", &self.consent_fingerprint)
            .field("authorization_required", &self.authorization_required)
            .finish()
    }
}

impl Drop for BookRegistryBinding {
    fn drop(&mut self) {
        self.question.zeroize();
    }
}

impl Default for BookPreparationRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(BookRegistryInner {
                state: Mutex::new(BookRegistryState::default()),
            }),
        }
    }
}

impl fmt::Debug for BookPreparationRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.state.lock();
        formatter
            .debug_struct("BookPreparationRegistry")
            .field("entry_count", &state.entries.len())
            .field("total_text_bytes", &state.total_text_bytes)
            .finish()
    }
}

impl BookPreparationRegistry {
    fn insert(
        &self,
        request: PreparedBookLearningRequest,
        summary: BookSummaryDraft,
    ) -> AppResult<BookLearningPreparationSummary> {
        self.insert_at(request, summary, Instant::now(), Utc::now())
    }

    fn insert_at(
        &self,
        request: PreparedBookLearningRequest,
        summary: BookSummaryDraft,
        now: Instant,
        wall_now: DateTime<Utc>,
    ) -> AppResult<BookLearningPreparationSummary> {
        let text_bytes = request_text_bytes(&request);
        if text_bytes > MAX_TEXT_BYTES_PER_PREPARATION {
            return Err(AppError::new(AppErrorCode::ContextTooLarge));
        }
        let expires_at = now
            .checked_add(BOOK_PREPARATION_TTL)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        let wall_expires_at = wall_now
            + chrono::Duration::from_std(BOOK_PREPARATION_TTL)
                .map_err(|_| AppError::new(AppErrorCode::RequestConflict))?;
        let mut state = self.inner.state.lock();
        gc_registry(&mut state, now);
        if state.entries.len() >= MAX_BOOK_PREPARATIONS
            || state.total_text_bytes.saturating_add(text_bytes) > MAX_TOTAL_TEXT_BYTES
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let preparation_id = unique_preparation_id(&state.entries);
        let authorized = !request.authorization_required;
        state.total_text_bytes = state.total_text_bytes.saturating_add(text_bytes);
        state.entries.insert(
            preparation_id,
            BookRegistryEntry {
                request,
                text_bytes,
                expires_at,
                authorized,
            },
        );
        Ok(BookLearningPreparationSummary {
            preparation_id,
            provider_display_name: summary.provider_display_name,
            profile_display_name: summary.profile_display_name,
            model_display_name: summary.model_display_name,
            estimated_input_tokens: summary.estimated_input_tokens,
            source_count: summary.source_count,
            citation_count: summary.citation_count,
            omitted_source_count: summary.omitted_source_count,
            risk_flags: summary.risk_flags,
            requires_blocking_confirmation: summary.requires_blocking_confirmation,
            expires_at: wall_expires_at,
        })
    }

    fn binding(&self, preparation_id: Uuid, now: Instant) -> AppResult<BookRegistryBinding> {
        let mut state = self.inner.state.lock();
        gc_registry(&mut state, now);
        let entry = state
            .entries
            .get(&preparation_id)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        Ok(BookRegistryBinding {
            preparation_id,
            book_id: entry.request.binding.book.book_id,
            target: entry.request.target.clone(),
            question: entry.request.current_question()?.to_owned(),
            profile_id: entry.request.binding.profile_id,
            snapshot_fingerprint: entry.request.snapshot_fingerprint.clone(),
            consent_fingerprint: entry.request.consent_fingerprint.clone(),
            authorization_required: entry.request.authorization_required,
        })
    }

    fn authorize(&self, preparation_id: Uuid, now: Instant) -> AppResult<()> {
        let mut state = self.inner.state.lock();
        gc_registry(&mut state, now);
        let entry = state
            .entries
            .get_mut(&preparation_id)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        if !entry.request.authorization_required || entry.authorized {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        entry.authorized = true;
        Ok(())
    }

    fn consume(
        &self,
        preparation_id: Uuid,
        now: Instant,
    ) -> AppResult<PreparedBookLearningRequest> {
        let mut state = self.inner.state.lock();
        gc_registry(&mut state, now);
        let entry = state
            .entries
            .get(&preparation_id)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        if !entry.authorized {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let entry = state
            .remove(preparation_id)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        Ok(entry.request)
    }

    pub fn discard(&self, preparation_id: Uuid) {
        self.inner.state.lock().remove(preparation_id);
    }

    pub fn invalidate(&self, request: &InvalidateLearningPreparations) -> AppResult<u32> {
        validate_invalidation(request)?;
        let mut state = self.inner.state.lock();
        gc_registry(&mut state, Instant::now());
        let ids = state
            .entries
            .iter()
            .filter_map(|(id, entry)| {
                let matches = match request.reason {
                    LearningInvalidationReason::RouteChange
                    | LearningInvalidationReason::DefaultChange => true,
                    LearningInvalidationReason::BookChange => {
                        Some(entry.request.binding.book.book_id) == request.book_id
                    }
                    LearningInvalidationReason::ProfileChange => {
                        Some(entry.request.binding.profile_id) == request.provider_profile_id
                    }
                };
                matches.then_some(*id)
            })
            .collect::<Vec<_>>();
        for id in &ids {
            state.remove(*id);
        }
        u32::try_from(ids.len()).map_err(|_| AppError::new(AppErrorCode::RequestConflict))
    }

    pub fn garbage_collect(&self) -> u32 {
        let mut state = self.inner.state.lock();
        let before = state.entries.len();
        gc_registry(&mut state, Instant::now());
        u32::try_from(before.saturating_sub(state.entries.len())).unwrap_or(u32::MAX)
    }

    #[cfg(test)]
    pub(crate) fn set_expiry_for_test(
        &self,
        preparation_id: Uuid,
        expires_at: Instant,
    ) -> AppResult<()> {
        let mut state = self.inner.state.lock();
        let entry = state
            .entries
            .get_mut(&preparation_id)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        entry.expires_at = expires_at;
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn stats(&self) -> (usize, usize) {
        let state = self.inner.state.lock();
        (state.entries.len(), state.total_text_bytes)
    }
}

fn gc_registry(state: &mut BookRegistryState, now: Instant) {
    let expired = state
        .entries
        .iter()
        .filter_map(|(id, entry)| (now >= entry.expires_at).then_some(*id))
        .collect::<Vec<_>>();
    for id in expired {
        state.remove(id);
    }
}

fn unique_preparation_id(entries: &BTreeMap<Uuid, BookRegistryEntry>) -> Uuid {
    loop {
        let candidate = Uuid::new_v4();
        if !entries.contains_key(&candidate) {
            return candidate;
        }
    }
}

fn request_text_bytes(request: &PreparedBookLearningRequest) -> usize {
    prepared_text_bytes(
        &request.binding,
        &request.teaching_instruction,
        &request.prepared_prompt,
        &request.packed_context,
    )
}

fn prepared_text_bytes(
    binding: &ProviderBindingSnapshot,
    teaching_instruction: &TeachingInstructionDto,
    prepared_prompt: &PreparedPrompt,
    packed_context: &PackedContext,
) -> usize {
    let mut total = prepared_prompt
        .messages
        .iter()
        .fold(prepared_prompt.system.len(), |total, message| {
            total.saturating_add(message.content.len())
        });
    total = total
        .saturating_add(teaching_instruction.instruction.len())
        .saturating_add(binding.book.source_sha256.len())
        .saturating_add(binding.book.updated_at.len())
        .saturating_add(binding.provider_display_name.len())
        .saturating_add(binding.profile_display_name.len())
        .saturating_add(binding.model_id.len())
        .saturating_add(binding.model_display_name.len())
        .saturating_add(binding.profile_updated_at.len());
    for segment in &packed_context.segments {
        total = total
            .saturating_add(segment.stable_id.len())
            .saturating_add(segment.content.len())
            .saturating_add(segment.locator_label.len());
        if let Some(seed) = &segment.citation_seed {
            total = total
                .saturating_add(seed.label.len())
                .saturating_add(locator_text_bytes(&seed.locator));
        }
        if let Some(citation) = &segment.citation {
            total = total.saturating_add(citation_text_bytes(citation));
        }
    }
    for citation in &packed_context.citations {
        total = total.saturating_add(citation_text_bytes(citation));
    }
    total
}

fn citation_text_bytes(citation: &Citation) -> usize {
    citation
        .id
        .len()
        .saturating_add(citation.label.len())
        .saturating_add(locator_text_bytes(&citation.locator))
}

fn locator_text_bytes(locator: &DocumentLocator) -> usize {
    match locator {
        DocumentLocator::Epub { cfi, .. } => cfi.len(),
        DocumentLocator::Pdf { .. } | DocumentLocator::Docx { .. } => 0,
    }
}

fn validate_invalidation(request: &InvalidateLearningPreparations) -> AppResult<()> {
    let valid = match request.reason {
        LearningInvalidationReason::RouteChange | LearningInvalidationReason::DefaultChange => {
            request.book_id.is_none() && request.provider_profile_id.is_none()
        }
        LearningInvalidationReason::BookChange => {
            request.book_id.is_some() && request.provider_profile_id.is_none()
        }
        LearningInvalidationReason::ProfileChange => {
            request.book_id.is_none() && request.provider_profile_id.is_some()
        }
    };
    if valid {
        Ok(())
    } else {
        Err(AppError::new(AppErrorCode::InvalidInput))
    }
}

async fn load_local_snapshot(
    pool: &SqlitePool,
    capabilities: &ProviderCapabilityRegistry,
    metadata: &PrepareBookLearningRequestMetadata,
) -> AppResult<LocalPreparationSnapshot> {
    let mut transaction = pool.begin().await?;
    let binding = load_provider_binding(&mut transaction, capabilities, metadata.book_id()).await?;
    let teaching_instruction = load_teaching_instruction(&mut transaction).await?;
    verify_content_ownership(&mut transaction, metadata.book_id()).await?;
    let (outline, omitted_outline_items) =
        load_bounded_outline(&mut transaction, &binding.book).await?;
    let (retrieval, omitted_retrieval_items) = load_bounded_retrieval(
        &mut transaction,
        metadata.book_id(),
        &binding.book.format,
        metadata.question(),
    )
    .await?;
    let history =
        load_bounded_book_history(&mut transaction, metadata, &binding.book.format).await?;
    let fingerprint = fingerprint_snapshot(
        &binding,
        &teaching_instruction,
        &history.target,
        &outline,
        &retrieval,
        &history.messages,
        &history.authority_revision,
        history.total_messages,
        omitted_outline_items,
        omitted_retrieval_items,
        history.was_omitted,
    )?;
    transaction.rollback().await?;
    Ok(LocalPreparationSnapshot {
        binding,
        teaching_instruction,
        target: history.target,
        outline,
        retrieval,
        history: history.messages,
        total_history_messages: history.total_messages,
        omitted_outline_items,
        omitted_retrieval_items,
        history_was_omitted: history.was_omitted,
        fingerprint,
    })
}

async fn load_provider_binding(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    capabilities: &ProviderCapabilityRegistry,
    book_id: Uuid,
) -> AppResult<ProviderBindingSnapshot> {
    let book =
        sqlx::query("SELECT id, format, import_status, sha256, updated_at FROM books WHERE id = ?")
            .bind(book_id.to_string())
            .fetch_optional(&mut **transaction)
            .await?
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if parse_uuid(book.try_get::<String, _>("id")?)? != book_id {
        return Err(database_error());
    }
    if book.try_get::<String, _>("import_status")? != "ready" {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    let source_sha256: String = book
        .try_get::<Option<String>, _>("sha256")?
        .ok_or_else(database_error)?;
    if !valid_sha256(&source_sha256) {
        return Err(database_error());
    }
    let book_updated_at: String = book.try_get("updated_at")?;
    parse_rfc3339_timestamp(&book_updated_at)?;
    let book = BookBindingSnapshot {
        book_id,
        format: parse_book_format(&book.try_get::<String, _>("format")?)?,
        source_sha256,
        updated_at: book_updated_at,
    };

    let settings = sqlx::query(
        "SELECT default_learning_profile_id, context_mode, ui_language FROM app_settings WHERE id = 1",
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(database_error)?;
    let profile_id = settings
        .try_get::<Option<String>, _>("default_learning_profile_id")?
        .map(parse_uuid)
        .transpose()?
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    let context_mode = match settings.try_get::<String, _>("context_mode")?.as_str() {
        "standard" => ContextMode::Standard,
        "long" => ContextMode::Long,
        _ => return Err(database_error()),
    };
    let ui_language = match settings.try_get::<String, _>("ui_language")?.as_str() {
        "zh-CN" => UiLanguage::ZhCn,
        "zh-TW" => UiLanguage::ZhTw,
        "en" => UiLanguage::En,
        _ => return Err(database_error()),
    };
    let profile = sqlx::query(
        "SELECT id, provider_kind, display_name, model_id, context_window_tokens, is_active, validated_at, updated_at FROM provider_profiles WHERE id = ?",
    )
    .bind(profile_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if parse_uuid(profile.try_get::<String, _>("id")?)? != profile_id
        || profile.try_get::<i64, _>("is_active")? != 1
    {
        return Err(database_error());
    }
    let provider_kind = parse_provider_kind(&profile.try_get::<String, _>("provider_kind")?)?;
    let model_id: String = profile.try_get("model_id")?;
    if model_id.trim() != model_id
        || model_id.is_empty()
        || model_id.chars().count() > MAX_MODEL_ID_CODE_POINTS
        || model_id.chars().any(char::is_control)
    {
        return Err(database_error());
    }
    if capabilities.operation_support(&provider_kind, &model_id, AiOperation::TextLearning)
        != CapabilitySupport::Supported
    {
        return Err(AppError::unsupported_provider_capability());
    }
    let provider = capabilities
        .capabilities()
        .iter()
        .find(|candidate| candidate.kind == provider_kind)
        .ok_or_else(AppError::unsupported_provider_capability)?;
    let model = provider
        .models
        .iter()
        .find(|candidate| candidate.id == model_id)
        .ok_or_else(AppError::unsupported_provider_capability)?;
    let profile_window = u32::try_from(profile.try_get::<i64, _>("context_window_tokens")?)
        .map_err(|_| database_error())?;
    let context_window_tokens = profile_window.min(model.context_window_tokens);
    if context_window_tokens == 0 {
        return Err(database_error());
    }
    let profile_display_name: String = profile.try_get("display_name")?;
    if !valid_display_name(&profile_display_name) {
        return Err(database_error());
    }
    let validated_at_text = profile
        .try_get::<Option<String>, _>("validated_at")?
        .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
    let validated_at = parse_rfc3339_timestamp(&validated_at_text)?;
    let profile_updated_at: String = profile.try_get("updated_at")?;
    parse_rfc3339_timestamp(&profile_updated_at)?;
    Ok(ProviderBindingSnapshot {
        book,
        profile_id,
        provider_kind,
        provider_display_name: provider.display_name.clone(),
        profile_display_name,
        model_id,
        model_display_name: model.display_name.clone(),
        context_window_tokens,
        default_max_output_tokens: model.default_max_output_tokens,
        validated_at,
        profile_updated_at,
        context_mode,
        ui_language,
    })
}

async fn load_teaching_instruction(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
) -> AppResult<TeachingInstructionDto> {
    let row = sqlx::query(
        "SELECT instruction, revision, updated_at FROM teaching_preferences WHERE id = 1",
    )
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(database_error)?;
    let instruction: String = row.try_get("instruction")?;
    crate::domain::validate_teaching_instruction(&instruction).map_err(|_| database_error())?;
    if !instruction.is_empty() && instruction.chars().all(char::is_whitespace) {
        return Err(database_error());
    }
    let revision =
        u64::try_from(row.try_get::<i64, _>("revision")?).map_err(|_| database_error())?;
    let updated_at_text: String = row.try_get("updated_at")?;
    let updated_at = parse_database_timestamp(&updated_at_text)?;
    Ok(TeachingInstructionDto {
        instruction,
        revision,
        updated_at,
    })
}

async fn verify_content_ownership(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
) -> AppResult<()> {
    let row = sqlx::query(
        r#"
SELECT
  (SELECT COUNT(*) FROM sections child
     LEFT JOIN sections parent ON parent.id = child.parent_id
   WHERE child.book_id = ? AND child.parent_id IS NOT NULL
     AND (parent.id IS NULL OR parent.book_id != child.book_id OR parent.id = child.id)) AS invalid_sections,
  (SELECT COUNT(*) FROM search_chunks chunk
     LEFT JOIN sections section ON section.id = chunk.section_id
   WHERE chunk.book_id = ? AND (section.id IS NULL OR section.book_id != chunk.book_id)) AS invalid_local_chunks,
  (SELECT COUNT(*) FROM index_pages page
     LEFT JOIN index_runs run ON run.id = page.run_id
   WHERE page.book_id = ? AND (run.id IS NULL OR run.book_id != page.book_id)) AS invalid_pages,
  (SELECT COUNT(*) FROM index_search_chunks chunk
     LEFT JOIN index_pages page ON page.id = chunk.page_id
     LEFT JOIN index_page_blocks block ON block.id = chunk.block_id
     LEFT JOIN index_corrections correction ON correction.id = chunk.correction_id
   WHERE chunk.book_id = ? AND (page.id IS NULL OR page.book_id != chunk.book_id
     OR block.id IS NULL OR block.book_id != chunk.book_id OR block.page_id != chunk.page_id
     OR (chunk.correction_id IS NOT NULL AND (correction.id IS NULL
       OR correction.book_id != chunk.book_id OR correction.page_id != chunk.page_id
       OR correction.target_block_id != chunk.block_id)))) AS invalid_index_chunks
"#,
    )
    .bind(book_id.to_string())
    .bind(book_id.to_string())
    .bind(book_id.to_string())
    .bind(book_id.to_string())
    .fetch_one(&mut **transaction)
    .await?;
    for column in [
        "invalid_sections",
        "invalid_local_chunks",
        "invalid_pages",
        "invalid_index_chunks",
    ] {
        if row.try_get::<i64, _>(column)? != 0 {
            return Err(database_error());
        }
    }
    Ok(())
}

async fn load_bounded_outline(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book: &BookBindingSnapshot,
) -> AppResult<(Vec<ContextCandidate>, u32)> {
    let total_i64: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sections WHERE book_id = ?")
        .bind(book.book_id.to_string())
        .fetch_one(&mut **transaction)
        .await?;
    let total = u32::try_from(total_i64).map_err(|_| database_error())?;
    let rows = sqlx::query(
        "SELECT id, book_id, parent_id, ordinal, title, locator_json FROM sections WHERE book_id = ? ORDER BY ordinal, id LIMIT ?",
    )
    .bind(book.book_id.to_string())
    .bind(i64::try_from(MAX_OUTLINE_ITEMS + 1).map_err(|_| database_error())?)
    .fetch_all(&mut **transaction)
    .await?;
    let mut candidates = Vec::new();
    let mut used_bytes = 0_usize;
    let mut used_tokens = 0_u64;
    for row in rows.into_iter().take(MAX_OUTLINE_ITEMS) {
        if parse_uuid(row.try_get::<String, _>("book_id")?)? != book.book_id {
            return Err(database_error());
        }
        let id = parse_uuid(row.try_get::<String, _>("id")?)?;
        let ordinal =
            u32::try_from(row.try_get::<i64, _>("ordinal")?).map_err(|_| database_error())?;
        if id != crate::domain::stable_section_id(book.book_id, ordinal) {
            return Err(database_error());
        }
        let title: String = row.try_get("title")?;
        if !valid_section_title(&title) {
            return Err(database_error());
        }
        let locator_json: String = row.try_get("locator_json")?;
        if locator_json.len() > MAX_SECTION_LOCATOR_JSON_BYTES {
            return Err(database_error());
        }
        let locator: DocumentLocator =
            serde_json::from_str(&locator_json).map_err(|_| database_error())?;
        if !locator_matches_format_and_section(&locator, &book.format, id) {
            return Err(database_error());
        }
        let label = format!("table of contents item {}", ordinal.saturating_add(1));
        let item_bytes = title.len().saturating_add(label.len());
        let item_tokens =
            conservative_token_count(&title).saturating_add(conservative_token_count(&label));
        if used_bytes.saturating_add(item_bytes) > MAX_OUTLINE_TOTAL_BYTES
            || used_tokens.saturating_add(item_tokens) > MAX_OUTLINE_TOKENS
        {
            break;
        }
        used_bytes = used_bytes.saturating_add(item_bytes);
        used_tokens = used_tokens.saturating_add(item_tokens);
        candidates.push(ContextCandidate {
            stable_id: format!("directory:{id}"),
            book_id: book.book_id,
            section_id: Some(id),
            source_kind: ContextSourceKind::Directory,
            source: ContextSource::Directory,
            same_section: false,
            relevance_micros: 0,
            ordinal,
            text: title,
            locator_label: label,
            locator: None,
            review_status: CitationReviewStatus::NotRequired,
            provenance_key: format!("directory:{ordinal}"),
            citation_seed: None,
        });
    }
    let retained = u32::try_from(candidates.len()).map_err(|_| database_error())?;
    Ok((candidates, total.saturating_sub(retained)))
}

async fn load_bounded_retrieval(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    book_id: Uuid,
    format: &BookFormat,
    question: &str,
) -> AppResult<(Vec<ContextCandidate>, u32)> {
    let hits =
        search_book_relevant_in_transaction(transaction, book_id, question, MAX_RETRIEVAL_FETCH)
            .await?;
    let hit_count = hits.len();
    let mut candidates = Vec::new();
    let mut used_bytes = 0_usize;
    let mut used_tokens = 0_u64;
    for hit in hits {
        let candidate = book_candidate_from_search_hit(book_id, hit)?;
        if !candidate
            .locator
            .as_ref()
            .is_some_and(|locator| locator_matches_format(locator, format))
            || candidate.section_id.is_some_and(|section_id| {
                matches!(
                    candidate.locator.as_ref(),
                    Some(DocumentLocator::Epub {
                        section_id: locator_section,
                        ..
                    }) if *locator_section != section_id
                )
            })
        {
            return Err(database_error());
        }
        let item_bytes = candidate
            .text
            .len()
            .saturating_add(candidate.locator_label.len());
        if item_bytes > MAX_RETRIEVAL_ITEM_BYTES {
            return Err(database_error());
        }
        let item_tokens = conservative_token_count(&candidate.text)
            .saturating_add(conservative_token_count(&candidate.locator_label));
        if candidates.len() >= MAX_RETRIEVAL_RESULTS
            || used_bytes.saturating_add(item_bytes) > MAX_RETRIEVAL_TOTAL_BYTES
            || used_tokens.saturating_add(item_tokens) > MAX_RETRIEVAL_TOKENS
        {
            break;
        }
        used_bytes = used_bytes.saturating_add(item_bytes);
        used_tokens = used_tokens.saturating_add(item_tokens);
        candidates.push(candidate);
    }
    let omitted = hit_count.saturating_sub(candidates.len());
    Ok((
        candidates,
        u32::try_from(omitted).map_err(|_| AppError::new(AppErrorCode::ContextTooLarge))?,
    ))
}

struct BoundedBookHistory {
    target: BookPreparationTarget,
    messages: Vec<UnifiedMessage>,
    authority_revision: String,
    total_messages: u32,
    was_omitted: bool,
}

async fn load_bounded_book_history(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    metadata: &PrepareBookLearningRequestMetadata,
    format: &BookFormat,
) -> AppResult<BoundedBookHistory> {
    let Some(conversation_id) = metadata.conversation_id() else {
        return Ok(BoundedBookHistory {
            target: BookPreparationTarget::New,
            messages: Vec::new(),
            authority_revision: "new-book-conversation".to_owned(),
            total_messages: 0,
            was_omitted: false,
        });
    };
    let book_id = metadata.book_id();
    let conversation = sqlx::query(
        "SELECT id, book_id, section_id, scope, anchor_kind, anchor_json, selected_text, created_at, updated_at FROM conversations WHERE id = ? AND book_id = ?",
    )
    .bind(conversation_id.to_string())
    .bind(book_id.to_string())
    .fetch_optional(&mut **transaction)
    .await?
    .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if parse_uuid(conversation.try_get::<String, _>("id")?)? != conversation_id
        || parse_uuid(conversation.try_get::<String, _>("book_id")?)? != book_id
        || conversation.try_get::<String, _>("scope")? != "book"
        || conversation
            .try_get::<Option<String>, _>("section_id")?
            .is_some()
        || conversation
            .try_get::<Option<String>, _>("anchor_kind")?
            .is_some()
        || conversation
            .try_get::<Option<String>, _>("anchor_json")?
            .is_some()
        || conversation
            .try_get::<Option<String>, _>("selected_text")?
            .is_some()
    {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let conversation_created_text: String = conversation.try_get("created_at")?;
    let conversation_updated_text: String = conversation.try_get("updated_at")?;
    let conversation_created = parse_database_timestamp(&conversation_created_text)?;
    let conversation_updated = parse_database_timestamp(&conversation_updated_text)?;
    if conversation_updated < conversation_created {
        return Err(database_error());
    }

    let aggregate = sqlx::query(
        "SELECT COUNT(*) AS message_count, MAX(ordinal) AS max_ordinal, MIN(created_at) AS first_created_at, MAX(created_at) AS last_created_at, SUM(CASE WHEN length(trim(content)) = 0 OR instr(content, char(0)) != 0 OR instr(content, char(13)) != 0 OR action NOT IN ('ask', 'continue') OR (ordinal < 2 AND action != 'ask') OR (ordinal >= 2 AND action != 'continue') OR (ordinal % 2 = 0 AND (role != 'user' OR provider_id IS NOT NULL OR model_id IS NOT NULL OR citations_json IS NOT NULL)) OR (ordinal % 2 = 1 AND (role != 'assistant' OR provider_id IS NULL OR model_id IS NULL OR citations_json IS NULL)) THEN 1 ELSE 0 END) AS invalid_count FROM messages WHERE conversation_id = ?",
    )
    .bind(conversation_id.to_string())
    .fetch_one(&mut **transaction)
    .await?;
    let count = aggregate.try_get::<i64, _>("message_count")?;
    let max_ordinal = aggregate.try_get::<Option<i64>, _>("max_ordinal")?;
    if count < 2
        || count % 2 != 0
        || max_ordinal != Some(count - 1)
        || aggregate.try_get::<i64, _>("invalid_count")? != 0
    {
        return Err(database_error());
    }
    let total_messages = u32::try_from(count).map_err(|_| database_error())?;
    let first_created_text = aggregate
        .try_get::<Option<String>, _>("first_created_at")?
        .ok_or_else(database_error)?;
    let last_created_text = aggregate
        .try_get::<Option<String>, _>("last_created_at")?
        .ok_or_else(database_error)?;
    let first_created = parse_database_timestamp(&first_created_text)?;
    let last_created = parse_database_timestamp(&last_created_text)?;
    if first_created < conversation_created || last_created > conversation_updated {
        return Err(database_error());
    }

    let mut rows = sqlx::query(
        "SELECT ordinal, role, action, content, provider_id, model_id, citations_json, created_at FROM messages WHERE conversation_id = ? ORDER BY ordinal DESC LIMIT ?",
    )
    .bind(conversation_id.to_string())
    .bind(i64::try_from(MAX_HISTORY_MESSAGES).map_err(|_| database_error())?)
    .fetch_all(&mut **transaction)
    .await?;
    rows.reverse();
    if !rows.len().is_multiple_of(2) {
        return Err(database_error());
    }
    let first_expected = count
        .checked_sub(i64::try_from(rows.len()).map_err(|_| database_error())?)
        .ok_or_else(database_error)?;
    let mut messages = Vec::with_capacity(rows.len());
    let mut prior_timestamp = None;
    for (offset, row) in rows.into_iter().enumerate() {
        let expected_ordinal = first_expected
            .checked_add(i64::try_from(offset).map_err(|_| database_error())?)
            .ok_or_else(database_error)?;
        let ordinal = row.try_get::<i64, _>("ordinal")?;
        if ordinal != expected_ordinal {
            return Err(database_error());
        }
        let role = if ordinal % 2 == 0 {
            validate_history_user_row(&row)?;
            UnifiedRole::User
        } else {
            validate_history_assistant_row(transaction, &row, book_id, format).await?;
            UnifiedRole::Assistant
        };
        let timestamp = parse_database_timestamp(&row.try_get::<String, _>("created_at")?)?;
        if prior_timestamp.is_some_and(|prior| prior > timestamp) {
            return Err(database_error());
        }
        prior_timestamp = Some(timestamp);
        let content: String = row.try_get("content")?;
        let content = if role == UnifiedRole::Assistant {
            super::history::strip_historical_citation_ids(&content)
        } else {
            content
        };
        messages.push(UnifiedMessage { role, content });
    }

    let loaded_count = messages.len();
    while history_cost(&messages).is_none_or(|(bytes, tokens)| {
        bytes > MAX_HISTORY_TOTAL_BYTES || tokens > MAX_HISTORY_TOKENS
    }) {
        if messages.len() < 2 {
            return Err(AppError::new(AppErrorCode::ContextTooLarge));
        }
        messages.drain(..2);
    }
    let was_omitted = usize::try_from(total_messages).map_err(|_| database_error())?
        > messages.len()
        || messages.len() < loaded_count;
    Ok(BoundedBookHistory {
        target: BookPreparationTarget::Continue {
            conversation_id,
            expected_next_ordinal: total_messages,
        },
        messages,
        authority_revision: format!(
            "{conversation_created_text}|{conversation_updated_text}|{count}|{first_created_text}|{last_created_text}"
        ),
        total_messages,
        was_omitted,
    })
}

fn validate_history_user_row(row: &SqliteRow) -> AppResult<()> {
    if row.try_get::<String, _>("role")? != "user"
        || row.try_get::<Option<String>, _>("provider_id")?.is_some()
        || row.try_get::<Option<String>, _>("model_id")?.is_some()
        || row
            .try_get::<Option<String>, _>("citations_json")?
            .is_some()
    {
        return Err(database_error());
    }
    let content: String = row.try_get("content")?;
    validate_question(&content).map_err(|_| database_error())
}

async fn validate_history_assistant_row(
    transaction: &mut sqlx::Transaction<'_, sqlx::Sqlite>,
    row: &SqliteRow,
    book_id: Uuid,
    format: &BookFormat,
) -> AppResult<()> {
    if row.try_get::<String, _>("role")? != "assistant" {
        return Err(database_error());
    }
    parse_uuid(
        row.try_get::<Option<String>, _>("provider_id")?
            .ok_or_else(database_error)?,
    )?;
    let model_id = row
        .try_get::<Option<String>, _>("model_id")?
        .ok_or_else(database_error)?;
    let content: String = row.try_get("content")?;
    if model_id.trim() != model_id
        || model_id.is_empty()
        || model_id.chars().count() > MAX_MODEL_ID_CODE_POINTS
        || model_id.chars().any(char::is_control)
        || content.trim().is_empty()
        || content.len() > crate::db::messages::MAX_LEARNING_ANSWER_BYTES
        || content.chars().count() > crate::db::messages::MAX_LEARNING_ANSWER_CODE_POINTS
        || contains_disallowed_control(&content)
    {
        return Err(database_error());
    }
    let citations_json = row
        .try_get::<Option<String>, _>("citations_json")?
        .ok_or_else(database_error)?;
    if citations_json.len() > MAX_HISTORY_CITATIONS_JSON_BYTES {
        return Err(database_error());
    }
    let citations: Vec<Citation> =
        serde_json::from_str(&citations_json).map_err(|_| database_error())?;
    if citations.len() > crate::db::messages::MAX_LEARNING_CITATIONS {
        return Err(database_error());
    }
    let mut section_ids = std::collections::BTreeSet::new();
    for citation in citations {
        if citation.book_id != book_id
            || !citation.quoteable
            || !locator_matches_format(&citation.locator, format)
        {
            return Err(database_error());
        }
        if let Some(section_id) = citation.section_id {
            section_ids.insert(section_id);
        }
    }
    for section_id in section_ids {
        let count: i64 =
            sqlx::query_scalar("SELECT COUNT(*) FROM sections WHERE id = ? AND book_id = ?")
                .bind(section_id.to_string())
                .bind(book_id.to_string())
                .fetch_one(&mut **transaction)
                .await?;
        if count != 1 {
            return Err(database_error());
        }
    }
    Ok(())
}

fn history_cost(messages: &[UnifiedMessage]) -> Option<(usize, u64)> {
    let mut bytes = 0_usize;
    let mut tokens = 0_u64;
    for message in messages {
        bytes = bytes.checked_add(message.content.len())?;
        tokens = tokens
            .checked_add(conservative_token_count(&message.content))?
            .checked_add(HISTORY_MESSAGE_FRAMING_TOKENS)?;
    }
    Some((bytes, tokens))
}

fn pack_local_snapshot(
    snapshot: &LocalPreparationSnapshot,
    question: &str,
) -> AppResult<(PreparedPrompt, PackedContext, u32)> {
    let budget = InputBudget::new(
        snapshot.binding.context_window_tokens,
        snapshot.binding.context_mode.clone(),
        snapshot.binding.default_max_output_tokens,
    );
    let mut history = snapshot.history.clone();
    let mut outline = snapshot.outline.clone();
    let mut retrieval = snapshot.retrieval.clone();
    loop {
        let mut candidates = outline.clone();
        candidates.extend(retrieval.clone());
        if snapshot.history_was_omitted || history.len() < snapshot.history.len() {
            candidates.push(ContextCandidate::history_placeholder(
                snapshot.binding.book.book_id,
                None,
            ));
        }
        let operation = match snapshot.target {
            BookPreparationTarget::New => PromptOperation::Ask,
            BookPreparationTarget::Continue { .. } => PromptOperation::Continue,
        };
        match PromptPolicy.pack_book_and_prepare(
            PromptInput {
                operation,
                expected_language: Some(snapshot.binding.ui_language.code().to_owned()),
                book_id: Some(snapshot.binding.book.book_id),
                teaching_instruction: snapshot.teaching_instruction.clone(),
                context_segments: Vec::new(),
                prior_messages: history.clone(),
                current_question: question.to_owned(),
                input_budget_tokens: u64::from(budget.usable_input),
            },
            budget,
            candidates,
        ) {
            Ok((prompt, mut packed)) => {
                let retained = u32::try_from(history.len())
                    .map_err(|_| AppError::new(AppErrorCode::ContextTooLarge))?;
                let manually_omitted = snapshot
                    .outline
                    .len()
                    .saturating_sub(outline.len())
                    .saturating_add(snapshot.retrieval.len().saturating_sub(retrieval.len()));
                packed.omitted_segment_count = packed.omitted_segment_count.saturating_add(
                    u32::try_from(manually_omitted)
                        .map_err(|_| AppError::new(AppErrorCode::ContextTooLarge))?,
                );
                return Ok((prompt, packed, retained));
            }
            Err(error) if error.code == AppErrorCode::ContextTooLarge && history.len() >= 2 => {
                // Reaching this branch means even the required directory/history payload did not
                // fit after the packer tried dropping every optional retrieval segment. Do not
                // reintroduce those lower-priority segments after removing the earliest pair.
                retrieval.clear();
                history.drain(..2);
            }
            Err(error) if error.code == AppErrorCode::ContextTooLarge && outline.len() > 1 => {
                retrieval.clear();
                outline.pop();
            }
            Err(error) => return Err(error),
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn fingerprint_snapshot(
    binding: &ProviderBindingSnapshot,
    teaching: &TeachingInstructionDto,
    target: &BookPreparationTarget,
    outline: &[ContextCandidate],
    retrieval: &[ContextCandidate],
    history: &[UnifiedMessage],
    history_authority_revision: &str,
    total_history_messages: u32,
    omitted_outline_items: u32,
    omitted_retrieval_items: u32,
    history_was_omitted: bool,
) -> AppResult<SnapshotFingerprint> {
    let mut fingerprint = FingerprintBuilder::new();
    fingerprint.text("textbooklens-book-preparation-v1");
    fingerprint.uuid(binding.book.book_id);
    fingerprint.text(book_format_name(&binding.book.format));
    fingerprint.text(&binding.book.source_sha256);
    fingerprint.text(&binding.book.updated_at);
    fingerprint.uuid(binding.profile_id);
    fingerprint.text(provider_kind_name(&binding.provider_kind));
    fingerprint.text(&binding.provider_display_name);
    fingerprint.text(&binding.profile_display_name);
    fingerprint.text(&binding.model_id);
    fingerprint.text(&binding.model_display_name);
    fingerprint.u32(binding.context_window_tokens);
    fingerprint.u32(binding.default_max_output_tokens);
    fingerprint.text(&binding.validated_at.to_rfc3339());
    fingerprint.text(&binding.profile_updated_at);
    fingerprint.text(match binding.context_mode {
        ContextMode::Standard => "standard",
        ContextMode::Long => "long",
    });
    fingerprint.text(binding.ui_language.code());
    fingerprint.text(&teaching.instruction);
    fingerprint.u64(teaching.revision);
    fingerprint.text(&teaching.updated_at.to_rfc3339());
    match target {
        BookPreparationTarget::New => fingerprint.text("new"),
        BookPreparationTarget::Continue {
            conversation_id,
            expected_next_ordinal,
        } => {
            fingerprint.text("continue");
            fingerprint.uuid(*conversation_id);
            fingerprint.u32(*expected_next_ordinal);
        }
    }
    fingerprint.candidates(outline)?;
    fingerprint.candidates(retrieval)?;
    fingerprint.text(history_authority_revision);
    fingerprint.u64(u64::try_from(history.len()).unwrap_or(u64::MAX));
    for message in history {
        fingerprint.text(match message.role {
            UnifiedRole::User => "user",
            UnifiedRole::Assistant => "assistant",
        });
        fingerprint.text(&message.content);
    }
    fingerprint.u32(total_history_messages);
    fingerprint.u32(omitted_outline_items);
    fingerprint.u32(omitted_retrieval_items);
    fingerprint.boolean(history_was_omitted);
    Ok(fingerprint.finish())
}

struct FingerprintBuilder(Sha256);

impl FingerprintBuilder {
    fn new() -> Self {
        Self(Sha256::new())
    }

    fn text(&mut self, value: &str) {
        self.u64(u64::try_from(value.len()).unwrap_or(u64::MAX));
        self.0.update(value.as_bytes());
    }

    fn uuid(&mut self, value: Uuid) {
        self.0.update(value.as_bytes());
    }

    fn u32(&mut self, value: u32) {
        self.0.update(value.to_be_bytes());
    }

    fn u64(&mut self, value: u64) {
        self.0.update(value.to_be_bytes());
    }

    fn boolean(&mut self, value: bool) {
        self.0.update([u8::from(value)]);
    }

    fn optional_uuid(&mut self, value: Option<Uuid>) {
        self.boolean(value.is_some());
        if let Some(value) = value {
            self.uuid(value);
        }
    }

    fn candidates(&mut self, candidates: &[ContextCandidate]) -> AppResult<()> {
        self.u64(u64::try_from(candidates.len()).unwrap_or(u64::MAX));
        for candidate in candidates {
            self.text(&candidate.stable_id);
            self.uuid(candidate.book_id);
            self.optional_uuid(candidate.section_id);
            self.text(context_source_kind_name(candidate.source_kind));
            self.text(context_source_name(candidate.source));
            self.boolean(candidate.same_section);
            self.u32(candidate.relevance_micros);
            self.u32(candidate.ordinal);
            self.text(&candidate.text);
            self.text(&candidate.locator_label);
            self.text(&serde_json::to_string(&candidate.locator).map_err(|_| database_error())?);
            self.text(citation_review_status_name(candidate.review_status));
            self.text(&candidate.provenance_key);
            self.boolean(candidate.citation_seed.is_some());
            if let Some(seed) = &candidate.citation_seed {
                self.uuid(seed.book_id);
                self.optional_uuid(seed.section_id);
                self.text(&serde_json::to_string(&seed.locator).map_err(|_| database_error())?);
                self.text(&seed.label);
                self.text(seed.source.as_str());
                self.text(citation_review_status_name(seed.review_status));
            }
        }
        Ok(())
    }

    fn finish(self) -> SnapshotFingerprint {
        SnapshotFingerprint(self.0.finalize().into())
    }
}

fn consent_snapshot(
    profile_id: Uuid,
    consents: Vec<ProviderOperationConsent>,
) -> AppResult<ConsentSnapshot> {
    if consents.len() != 3 {
        return Err(database_error());
    }
    let mut image_send = None;
    let mut ai_index = None;
    let mut cost_risk = None;
    let mut material = Zeroizing::new(Vec::with_capacity(256));
    for consent in consents {
        if consent.profile_id != profile_id {
            return Err(database_error());
        }
        let (slot, category) = match consent.category {
            ProviderOperationConsentCategory::ImageSend => {
                (&mut image_send, b"image_send".as_slice())
            }
            ProviderOperationConsentCategory::AiIndex => (&mut ai_index, b"ai_index".as_slice()),
            ProviderOperationConsentCategory::CostRisk => (&mut cost_risk, b"cost_risk".as_slice()),
        };
        if slot.replace(consent.decision).is_some() {
            return Err(database_error());
        }
        material.extend_from_slice(category);
        material.push(b'|');
        material.extend_from_slice(match consent.decision {
            ProviderOperationConsentDecision::Ask => b"ask",
            ProviderOperationConsentDecision::SkipPrompt => b"skip_prompt",
        });
        material.push(b'|');
        material.extend_from_slice(consent.updated_at.to_rfc3339().as_bytes());
        material.push(b';');
    }
    let _ = image_send.ok_or_else(database_error)?;
    let _ = ai_index.ok_or_else(database_error)?;
    Ok(ConsentSnapshot {
        fingerprint: ConsentFingerprint(Sha256::digest(material.as_slice()).into()),
        cost_risk: cost_risk.ok_or_else(database_error)?,
    })
}

fn parse_uuid(value: String) -> AppResult<Uuid> {
    Uuid::parse_str(&value).map_err(|_| database_error())
}

fn parse_database_timestamp(value: &str) -> AppResult<DateTime<Utc>> {
    let parsed = DateTime::parse_from_rfc3339(value)
        .map_err(|_| database_error())?
        .with_timezone(&Utc);
    if parsed.to_rfc3339_opts(SecondsFormat::Millis, true) != value {
        return Err(database_error());
    }
    Ok(parsed)
}

fn parse_rfc3339_timestamp(value: &str) -> AppResult<DateTime<Utc>> {
    DateTime::parse_from_rfc3339(value)
        .map(|timestamp| timestamp.with_timezone(&Utc))
        .map_err(|_| database_error())
}

fn parse_book_format(value: &str) -> AppResult<BookFormat> {
    match value {
        "pdf" => Ok(BookFormat::Pdf),
        "epub" => Ok(BookFormat::Epub),
        "docx" => Ok(BookFormat::Docx),
        _ => Err(database_error()),
    }
}

fn parse_provider_kind(value: &str) -> AppResult<ProviderKind> {
    match value {
        "openai" => Ok(ProviderKind::OpenAi),
        "gemini" => Ok(ProviderKind::Gemini),
        "anthropic" => Ok(ProviderKind::Anthropic),
        "deepseek" => Ok(ProviderKind::DeepSeek),
        "kimi" => Ok(ProviderKind::Kimi),
        _ => Err(database_error()),
    }
}

fn book_format_name(value: &BookFormat) -> &'static str {
    match value {
        BookFormat::Pdf => "pdf",
        BookFormat::Epub => "epub",
        BookFormat::Docx => "docx",
    }
}

fn provider_kind_name(value: &ProviderKind) -> &'static str {
    match value {
        ProviderKind::OpenAi => "openai",
        ProviderKind::Gemini => "gemini",
        ProviderKind::Anthropic => "anthropic",
        ProviderKind::DeepSeek => "deepseek",
        ProviderKind::Kimi => "kimi",
    }
}

fn context_source_kind_name(value: ContextSourceKind) -> &'static str {
    match value {
        ContextSourceKind::Selection => "selection",
        ContextSourceKind::Region => "region",
        ContextSourceKind::Neighbor => "neighbor",
        ContextSourceKind::Heading => "heading",
        ContextSourceKind::Definition => "definition",
        ContextSourceKind::TextbookSearch => "textbook_search",
        ContextSourceKind::UserNote => "user_note",
        ContextSourceKind::HistorySummary => "history_summary",
        ContextSourceKind::Directory => "directory",
    }
}

fn context_source_name(value: ContextSource) -> &'static str {
    match value {
        ContextSource::LocalText => "local_text",
        ContextSource::AiTranscribed => "ai_transcribed",
        ContextSource::AiDescription => "ai_description",
        ContextSource::UserCorrected => "user_corrected",
        ContextSource::UserNote => "user_note",
        ContextSource::HistorySummary => "history_summary",
        ContextSource::Directory => "directory",
    }
}

fn citation_review_status_name(value: CitationReviewStatus) -> &'static str {
    match value {
        CitationReviewStatus::NotRequired => "not_required",
        CitationReviewStatus::Indexed => "indexed",
        CitationReviewStatus::NeedsReview => "needs_review",
        CitationReviewStatus::UserCorrected => "user_corrected",
    }
}

fn locator_matches_format_and_section(
    locator: &DocumentLocator,
    format: &BookFormat,
    section_id: Uuid,
) -> bool {
    match (locator, format) {
        (DocumentLocator::Pdf { .. }, BookFormat::Pdf) => true,
        (
            DocumentLocator::Epub {
                section_id: locator_section,
                ..
            },
            BookFormat::Epub,
        ) => *locator_section == section_id,
        (DocumentLocator::Docx { .. }, BookFormat::Docx) => true,
        _ => false,
    }
}

fn locator_matches_format(locator: &DocumentLocator, format: &BookFormat) -> bool {
    matches!(
        (locator, format),
        (DocumentLocator::Pdf { .. }, BookFormat::Pdf)
            | (DocumentLocator::Epub { .. }, BookFormat::Epub)
            | (DocumentLocator::Docx { .. }, BookFormat::Docx)
    )
}

fn valid_section_title(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= MAX_SECTION_TITLE_BYTES
        && value.chars().count() <= MAX_SECTION_TITLE_CODE_POINTS
        && !value
            .chars()
            .any(|character| character.is_control() && character != '\t')
}

fn valid_display_name(value: &str) -> bool {
    !value.is_empty()
        && value.trim() == value
        && value.len() <= MAX_DISPLAY_NAME_BYTES
        && value.chars().count() <= MAX_DISPLAY_NAME_CODE_POINTS
        && !value.chars().any(char::is_control)
}

fn contains_disallowed_control(value: &str) -> bool {
    value
        .chars()
        .any(|character| character.is_control() && character != '\n' && character != '\t')
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn zeroize_candidate(candidate: &mut ContextCandidate) {
    candidate.stable_id.zeroize();
    candidate.text.zeroize();
    candidate.locator_label.zeroize();
    candidate.provenance_key.zeroize();
    if let Some(locator) = &mut candidate.locator {
        zeroize_document_locator(locator);
    }
    if let Some(seed) = &mut candidate.citation_seed {
        seed.label.zeroize();
        zeroize_document_locator(&mut seed.locator);
    }
}

fn zeroize_citation(citation: &mut Citation) {
    citation.id.zeroize();
    citation.label.zeroize();
    zeroize_document_locator(&mut citation.locator);
}

fn zeroize_document_locator(locator: &mut DocumentLocator) {
    if let DocumentLocator::Epub { cfi, .. } = locator {
        cfi.zeroize();
    }
}

fn database_error() -> AppError {
    AppError::new(AppErrorCode::DatabaseError)
}

#[cfg(test)]
#[path = "book_preparation_test.rs"]
mod tests;
