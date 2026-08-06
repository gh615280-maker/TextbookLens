use std::{
    collections::BTreeMap,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use sqlx::{Row, SqlitePool};
use uuid::Uuid;
use zeroize::{Zeroize, Zeroizing};

use crate::{
    ai::registry::ProviderCapabilityRegistry,
    credentials::CredentialStore,
    db::{providers, settings, teaching},
    domain::{
        AiOperation, CapabilitySupport, ContentAnchor, ContextMode, DocumentLocator,
        NormalizedRect, ProviderKind, ProviderOperationConsent, ProviderOperationConsentCategory,
        ProviderOperationConsentDecision, RegionAnchor, RegionLocator, TeachingInstructionDto,
    },
    errors::{AppError, AppErrorCode, AppResult},
    retrieval::{
        budget::{InputBudget, STANDARD_CONTEXT_CAP},
        context::{PackedContext, SelectionContextQuery, retrieve_selection_context},
    },
};

use super::captures::{
    ExpectedRegionCapture, OwnedCaptureBytes, RegionCaptureLimits, RegionCaptureMetadata,
    StagedRegionCapture, validate_and_stage_region_capture,
};
use super::{PreparedPrompt, PromptInput, PromptOperation, PromptPolicy};

pub const PREPARATION_TTL: Duration = Duration::from_secs(5 * 60);
const MAX_PREPARATIONS: usize = 64;
const MAX_TEXT_BYTES_PER_PREPARATION: usize = 2 * 1024 * 1024;
const MAX_TOTAL_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_TOTAL_IMAGE_BYTES: usize = 16 * 1024 * 1024;
const MAX_SELECTED_TEXT_CODE_POINTS: usize = 1_048_576;
const MAX_QUESTION_CODE_POINTS: usize = 16_384;
const MAX_TARGET_LANGUAGE_CODE_POINTS: usize = 128;
const MAX_MODEL_ID_CODE_POINTS: usize = 512;
const MAX_EPUB_CFI_CODE_POINTS: usize = 4_096;
const MAX_QUOTE_CONTEXT_CODE_POINTS: usize = 64;
const MAX_PDF_RECT_PAGES: usize = 1_024;
const MAX_PDF_RECTS_PER_PAGE: usize = 128;
const MAX_PDF_RECTS_TOTAL: usize = 1_024;
const VISUAL_REGION_PLACEHOLDER: &str =
    "Selected visual region; image content will be attached only after explicit authorization.";

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningAction {
    Explain,
    Example,
    Derive,
    Translate,
    Ask,
}

impl LearningAction {
    const fn prompt_operation(self) -> PromptOperation {
        match self {
            Self::Explain => PromptOperation::Explain,
            Self::Example => PromptOperation::Example,
            Self::Derive => PromptOperation::Derive,
            Self::Translate => PromptOperation::Translate,
            Self::Ask => PromptOperation::Ask,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningContentKind {
    TextSelection,
    ReliableTextRegion,
    VisualRegion,
}

impl LearningContentKind {
    const fn will_send_image(self) -> bool {
        matches!(self, Self::VisualRegion)
    }
}

#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct PrepareLearningRequestMetadata {
    pub book_id: Uuid,
    pub section_id: Uuid,
    pub provider_profile_id: Uuid,
    pub model_id: String,
    pub action: LearningAction,
    pub content_kind: LearningContentKind,
    pub anchor: ContentAnchor,
    pub selected_text: Option<String>,
    pub question: Option<String>,
    pub target_language: Option<String>,
}

impl fmt::Debug for PrepareLearningRequestMetadata {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PrepareLearningRequestMetadata")
            .field("book_id", &"<redacted>")
            .field("section_id", &"<redacted>")
            .field("provider_profile_id", &"<redacted>")
            .field("model_id", &"<redacted>")
            .field("action", &self.action)
            .field("content_kind", &self.content_kind)
            .field("anchor", &"<redacted>")
            .field(
                "selected_text_code_points",
                &self
                    .selected_text
                    .as_ref()
                    .map(|value| value.chars().count()),
            )
            .field(
                "question_code_points",
                &self.question.as_ref().map(|value| value.chars().count()),
            )
            .field(
                "target_language",
                &self.target_language.as_ref().map(|_| "<redacted>"),
            )
            .finish()
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreparationRiskFlag {
    ImageSend,
    CostRisk,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PreparationSummary {
    pub preparation_id: Uuid,
    pub provider_display_name: String,
    pub profile_display_name: String,
    pub model_display_name: String,
    pub estimated_input_tokens: u32,
    pub source_count: u32,
    pub citation_count: u32,
    pub omitted_source_count: u32,
    pub will_send_image: bool,
    pub risk_flags: Vec<PreparationRiskFlag>,
    pub requires_blocking_confirmation: bool,
    pub expires_at: DateTime<Utc>,
    pub action_category: LearningAction,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningAuthorizationDecision {
    Allow,
    Deny,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LearningInvalidationReason {
    RouteChange,
    BookChange,
    ProfileChange,
    DefaultChange,
}

#[derive(Clone, Debug, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct InvalidateLearningPreparations {
    pub reason: LearningInvalidationReason,
    pub book_id: Option<Uuid>,
    pub provider_profile_id: Option<Uuid>,
}

#[derive(Clone, PartialEq, Eq)]
struct BookBindingSnapshot {
    book_id: Uuid,
    format: String,
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
    section_id: Uuid,
    profile_id: Uuid,
    provider_kind: ProviderKind,
    provider_display_name: String,
    profile_display_name: String,
    model_id: String,
    model_display_name: String,
    context_window_tokens: u32,
    default_max_output_tokens: u32,
    validated_at: DateTime<Utc>,
    context_mode: ContextMode,
    requires_vision: bool,
    capture_limits: Option<RegionCaptureLimits>,
}

impl fmt::Debug for ProviderBindingSnapshot {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ProviderBindingSnapshot")
            .field("book", &self.book)
            .field("section_id", &"<redacted>")
            .field("profile_id", &"<redacted>")
            .field("provider_kind", &self.provider_kind)
            .field("provider_display_name", &self.provider_display_name)
            .field("profile_display_name", &self.profile_display_name)
            .field("model_id", &"<redacted>")
            .field("model_display_name", &self.model_display_name)
            .field("context_window_tokens", &self.context_window_tokens)
            .field("default_max_output_tokens", &self.default_max_output_tokens)
            .field("validated_at", &"<redacted>")
            .field("context_mode", &self.context_mode)
            .field("requires_vision", &self.requires_vision)
            .field("capture_limits", &self.capture_limits)
            .finish()
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
    image_send: ProviderOperationConsentDecision,
    cost_risk: ProviderOperationConsentDecision,
}

pub struct PreparedLearningRequest {
    binding: ProviderBindingSnapshot,
    action: LearningAction,
    content_kind: LearningContentKind,
    anchor: ContentAnchor,
    teaching_instruction: TeachingInstructionDto,
    prepared_prompt: PreparedPrompt,
    packed_context: PackedContext,
    consent_fingerprint: ConsentFingerprint,
    risk_flags: Vec<PreparationRiskFlag>,
    capture: Option<StagedRegionCapture>,
}

impl PreparedLearningRequest {
    pub fn provider_profile_id(&self) -> Uuid {
        self.binding.profile_id
    }

    pub fn model_id(&self) -> &str {
        &self.binding.model_id
    }

    pub fn book_id(&self) -> Uuid {
        self.binding.book.book_id
    }

    pub fn section_id(&self) -> Uuid {
        self.binding.section_id
    }

    pub fn default_max_output_tokens(&self) -> u32 {
        self.binding.default_max_output_tokens
    }

    pub fn requires_vision(&self) -> bool {
        self.binding.requires_vision
    }

    pub fn action(&self) -> LearningAction {
        self.action
    }

    pub fn anchor(&self) -> &ContentAnchor {
        &self.anchor
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

    pub fn capture(&self) -> Option<&StagedRegionCapture> {
        self.capture.as_ref()
    }

    pub fn current_question(&self) -> AppResult<&str> {
        match self.prepared_prompt.messages.as_slice() {
            [message] if message.role == crate::domain::UnifiedRole::User => {
                Ok(message.content.as_str())
            }
            _ => Err(AppError::new(AppErrorCode::RequestConflict)),
        }
    }
}

impl fmt::Debug for PreparedLearningRequest {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("PreparedLearningRequest")
            .field("binding", &self.binding)
            .field("action", &self.action)
            .field("content_kind", &self.content_kind)
            .field("anchor", &"<redacted>")
            .field("teaching_instruction", &"<redacted>")
            .field("prepared_prompt", &self.prepared_prompt)
            .field("packed_context", &self.packed_context)
            .field("consent_fingerprint", &self.consent_fingerprint)
            .field("risk_flags", &self.risk_flags)
            .field("has_capture", &self.capture.is_some())
            .finish()
    }
}

#[async_trait]
pub(crate) trait PreparationSensitiveAccess: Send + Sync {
    async fn require_credential(&self, profile_id: Uuid) -> AppResult<()>;

    async fn load_consents(
        &self,
        pool: &SqlitePool,
        profile_id: Uuid,
    ) -> AppResult<Vec<ProviderOperationConsent>>;
}

struct RuntimePreparationSensitiveAccess {
    credential_store: Arc<dyn CredentialStore>,
}

#[async_trait]
impl PreparationSensitiveAccess for RuntimePreparationSensitiveAccess {
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
pub struct LearningPreparationService {
    pool: SqlitePool,
    registry: PreparationRegistry,
    capabilities: ProviderCapabilityRegistry,
    sensitive_access: Arc<dyn PreparationSensitiveAccess>,
}

impl LearningPreparationService {
    pub fn new(
        pool: SqlitePool,
        registry: PreparationRegistry,
        capabilities: ProviderCapabilityRegistry,
        credential_store: Arc<dyn CredentialStore>,
    ) -> Self {
        Self {
            pool,
            registry,
            capabilities,
            sensitive_access: Arc::new(RuntimePreparationSensitiveAccess { credential_store }),
        }
    }

    #[cfg(test)]
    pub(crate) fn with_sensitive_access(
        mut self,
        sensitive_access: Arc<dyn PreparationSensitiveAccess>,
    ) -> Self {
        self.sensitive_access = sensitive_access;
        self
    }

    pub async fn prepare(
        &self,
        metadata: PrepareLearningRequestMetadata,
    ) -> AppResult<PreparationSummary> {
        validate_metadata(&metadata)?;
        let selected_text = validated_selected_text(&metadata)?;
        let current_question = current_question(&metadata)?;
        let query_text = if matches!(metadata.action, LearningAction::Ask) {
            current_question.clone()
        } else {
            selected_text
                .chars()
                .take(MAX_QUESTION_CODE_POINTS)
                .collect()
        };

        let initial_binding = load_binding(
            &self.pool,
            &self.capabilities,
            metadata.book_id,
            metadata.section_id,
            metadata.provider_profile_id,
            &metadata.model_id,
            metadata.content_kind.will_send_image(),
        )
        .await?;
        validate_section_scope(&self.pool, &initial_binding, &metadata.anchor).await?;

        let instruction = teaching::get_teaching_instruction(&self.pool).await?;
        let candidates = retrieve_selection_context(
            &self.pool,
            &SelectionContextQuery {
                book_id: metadata.book_id,
                section_id: metadata.section_id,
                anchor: metadata.anchor.clone(),
                selected_text,
                query_text,
            },
        )
        .await?;
        let budget = InputBudget::new(
            initial_binding.context_window_tokens,
            initial_binding.context_mode.clone(),
            initial_binding.default_max_output_tokens,
        );
        let (prepared_prompt, packed_context) = PromptPolicy.pack_and_prepare(
            PromptInput {
                operation: metadata.action.prompt_operation(),
                book_id: Some(metadata.book_id),
                teaching_instruction: instruction.clone(),
                context_segments: Vec::new(),
                prior_messages: Vec::new(),
                current_question,
                input_budget_tokens: u64::from(budget.usable_input),
            },
            budget,
            candidates,
        )?;

        // Everything below this barrier may touch credential, consent, or image/provider-adjacent
        // state. Mandatory retrieval and prompt packing above must succeed first.
        self.require_binding_unchanged(&initial_binding).await?;
        self.sensitive_access
            .require_credential(initial_binding.profile_id)
            .await?;
        let consent = consent_snapshot(
            initial_binding.profile_id,
            self.sensitive_access
                .load_consents(&self.pool, initial_binding.profile_id)
                .await?,
        )?;
        self.require_binding_unchanged(&initial_binding).await?;

        let mut risk_flags = Vec::new();
        if metadata.content_kind.will_send_image() {
            risk_flags.push(PreparationRiskFlag::ImageSend);
        }
        if packed_context.estimated_input_tokens > STANDARD_CONTEXT_CAP {
            risk_flags.push(PreparationRiskFlag::CostRisk);
        }
        let requires_blocking_confirmation = risk_flags.iter().any(|risk| match risk {
            PreparationRiskFlag::ImageSend => {
                consent.image_send == ProviderOperationConsentDecision::Ask
            }
            PreparationRiskFlag::CostRisk => {
                consent.cost_risk == ProviderOperationConsentDecision::Ask
            }
        });

        let source_count = u32::try_from(packed_context.segments.len())
            .map_err(|_| AppError::new(AppErrorCode::ContextTooLarge))?;
        let citation_count = u32::try_from(packed_context.citations.len())
            .map_err(|_| AppError::new(AppErrorCode::ContextTooLarge))?;
        let summary = SummaryDraft {
            provider_display_name: initial_binding.provider_display_name.clone(),
            profile_display_name: initial_binding.profile_display_name.clone(),
            model_display_name: initial_binding.model_display_name.clone(),
            estimated_input_tokens: packed_context.estimated_input_tokens,
            source_count,
            citation_count,
            omitted_source_count: packed_context.omitted_segment_count,
            will_send_image: metadata.content_kind.will_send_image(),
            risk_flags: risk_flags.clone(),
            requires_blocking_confirmation,
            action_category: metadata.action,
        };
        let request = PreparedLearningRequest {
            binding: initial_binding,
            action: metadata.action,
            content_kind: metadata.content_kind,
            anchor: metadata.anchor,
            teaching_instruction: instruction,
            prepared_prompt,
            packed_context,
            consent_fingerprint: consent.fingerprint,
            risk_flags,
            capture: None,
        };
        self.registry.insert(request, summary)
    }

    pub async fn authorize(
        &self,
        preparation_id: Uuid,
        decision: LearningAuthorizationDecision,
    ) -> AppResult<Option<Uuid>> {
        if decision == LearningAuthorizationDecision::Deny {
            self.registry.discard(preparation_id);
            return Ok(None);
        }
        let binding = self.registry.binding(preparation_id, Instant::now())?;
        if binding.risk_flags.is_empty() {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        self.revalidate_registry_binding(&binding).await?;
        self.registry
            .authorize(preparation_id, &binding.consent_fingerprint, Instant::now())
            .map(Some)
    }

    pub(crate) async fn stage_region_capture(
        &self,
        metadata: RegionCaptureMetadata,
        bytes: OwnedCaptureBytes,
    ) -> AppResult<()> {
        let binding = self
            .registry
            .binding(metadata.preparation_id, Instant::now())?;
        if !binding.will_send_image || binding.operation_token != Some(metadata.operation_token) {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        self.revalidate_registry_binding(&binding).await?;
        self.registry
            .stage_region_capture(metadata, bytes, Instant::now())
    }

    pub async fn consume(
        &self,
        preparation_id: Uuid,
        operation_token: Option<Uuid>,
    ) -> AppResult<PreparedLearningRequest> {
        let binding = self.registry.binding(preparation_id, Instant::now())?;
        if binding.operation_token != operation_token {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        self.revalidate_registry_binding(&binding).await?;
        self.registry
            .consume(preparation_id, operation_token, Instant::now())
    }

    /// Consumes the already-authorized preparation from the in-process execution boundary.
    /// The authorization token never crosses the public start command or appears in logs.
    pub async fn consume_for_execution(
        &self,
        preparation_id: Uuid,
    ) -> AppResult<PreparedLearningRequest> {
        let binding = self.registry.binding(preparation_id, Instant::now())?;
        self.revalidate_registry_binding(&binding).await?;
        self.registry
            .consume(preparation_id, binding.operation_token, Instant::now())
    }

    async fn require_binding_unchanged(&self, expected: &ProviderBindingSnapshot) -> AppResult<()> {
        let current = load_binding(
            &self.pool,
            &self.capabilities,
            expected.book.book_id,
            expected.section_id,
            expected.profile_id,
            &expected.model_id,
            expected.requires_vision,
        )
        .await
        .map_err(|_| AppError::new(AppErrorCode::RequestConflict))?;
        if &current != expected {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        Ok(())
    }

    async fn revalidate_registry_binding(&self, binding: &RegistryBinding) -> AppResult<()> {
        if let Err(error) = self.require_binding_unchanged(&binding.provider).await {
            self.registry.discard(binding.preparation_id);
            return Err(error);
        }
        if let Err(error) = self
            .sensitive_access
            .require_credential(binding.provider.profile_id)
            .await
        {
            self.registry.discard(binding.preparation_id);
            return Err(error);
        }
        let consents = match self
            .sensitive_access
            .load_consents(&self.pool, binding.provider.profile_id)
            .await
        {
            Ok(consents) => consents,
            Err(error) => {
                self.registry.discard(binding.preparation_id);
                return Err(error);
            }
        };
        let consent = match consent_snapshot(binding.provider.profile_id, consents) {
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
        if let Err(error) = self.require_binding_unchanged(&binding.provider).await {
            self.registry.discard(binding.preparation_id);
            return Err(error);
        }
        Ok(())
    }
}

#[derive(Clone)]
pub struct PreparationRegistry {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    state: Mutex<RegistryState>,
}

impl Drop for RegistryInner {
    fn drop(&mut self) {
        self.state.get_mut().clear();
    }
}

#[derive(Default)]
struct RegistryState {
    entries: BTreeMap<Uuid, RegistryEntry>,
    total_text_bytes: usize,
    total_image_bytes: usize,
}

impl RegistryState {
    fn clear(&mut self) {
        self.entries.clear();
        self.total_text_bytes = 0;
        self.total_image_bytes = 0;
    }

    fn remove(&mut self, preparation_id: Uuid) -> Option<RegistryEntry> {
        let entry = self.entries.remove(&preparation_id)?;
        self.total_text_bytes = self.total_text_bytes.saturating_sub(entry.text_bytes);
        self.total_image_bytes = self.total_image_bytes.saturating_sub(entry.image_bytes());
        Some(entry)
    }
}

struct RegistryEntry {
    request: PreparedLearningRequest,
    summary: PreparationSummary,
    text_bytes: usize,
    expires_at: Instant,
    operation_token: Option<Uuid>,
}

impl RegistryEntry {
    fn image_bytes(&self) -> usize {
        self.request
            .capture
            .as_ref()
            .map_or(0, |capture| capture.bytes().len())
    }
}

struct SummaryDraft {
    provider_display_name: String,
    profile_display_name: String,
    model_display_name: String,
    estimated_input_tokens: u32,
    source_count: u32,
    citation_count: u32,
    omitted_source_count: u32,
    will_send_image: bool,
    risk_flags: Vec<PreparationRiskFlag>,
    requires_blocking_confirmation: bool,
    action_category: LearningAction,
}

#[derive(Clone)]
struct RegistryBinding {
    preparation_id: Uuid,
    provider: ProviderBindingSnapshot,
    consent_fingerprint: ConsentFingerprint,
    risk_flags: Vec<PreparationRiskFlag>,
    will_send_image: bool,
    operation_token: Option<Uuid>,
}

impl fmt::Debug for RegistryBinding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("RegistryBinding")
            .field("preparation_id", &"<redacted>")
            .field("provider", &self.provider)
            .field("consent_fingerprint", &self.consent_fingerprint)
            .field("risk_flags", &self.risk_flags)
            .field("will_send_image", &self.will_send_image)
            .field("has_operation_token", &self.operation_token.is_some())
            .finish()
    }
}

impl Default for PreparationRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                state: Mutex::new(RegistryState::default()),
            }),
        }
    }
}

impl fmt::Debug for PreparationRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.state.lock();
        formatter
            .debug_struct("PreparationRegistry")
            .field("entry_count", &state.entries.len())
            .field("total_text_bytes", &state.total_text_bytes)
            .field("total_image_bytes", &state.total_image_bytes)
            .finish()
    }
}

impl PreparationRegistry {
    fn insert(
        &self,
        request: PreparedLearningRequest,
        summary: SummaryDraft,
    ) -> AppResult<PreparationSummary> {
        self.insert_at(request, summary, Instant::now(), Utc::now())
    }

    fn insert_at(
        &self,
        request: PreparedLearningRequest,
        summary: SummaryDraft,
        now: Instant,
        wall_now: DateTime<Utc>,
    ) -> AppResult<PreparationSummary> {
        let text_bytes = request_text_bytes(&request);
        if text_bytes > MAX_TEXT_BYTES_PER_PREPARATION {
            return Err(AppError::new(AppErrorCode::ContextTooLarge));
        }
        let expires_at = now
            .checked_add(PREPARATION_TTL)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        let wall_expires_at = wall_now
            + chrono::Duration::from_std(PREPARATION_TTL)
                .map_err(|_| AppError::new(AppErrorCode::RequestConflict))?;
        let mut state = self.inner.state.lock();
        gc_locked(&mut state, now);
        if state.entries.len() >= MAX_PREPARATIONS
            || state.total_text_bytes.saturating_add(text_bytes) > MAX_TOTAL_TEXT_BYTES
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let preparation_id = unique_id(&state.entries);
        let summary = PreparationSummary {
            preparation_id,
            provider_display_name: summary.provider_display_name,
            profile_display_name: summary.profile_display_name,
            model_display_name: summary.model_display_name,
            estimated_input_tokens: summary.estimated_input_tokens,
            source_count: summary.source_count,
            citation_count: summary.citation_count,
            omitted_source_count: summary.omitted_source_count,
            will_send_image: summary.will_send_image,
            risk_flags: summary.risk_flags,
            requires_blocking_confirmation: summary.requires_blocking_confirmation,
            expires_at: wall_expires_at,
            action_category: summary.action_category,
        };
        state.total_text_bytes = state.total_text_bytes.saturating_add(text_bytes);
        state.entries.insert(
            preparation_id,
            RegistryEntry {
                request,
                summary: summary.clone(),
                text_bytes,
                expires_at,
                operation_token: None,
            },
        );
        Ok(summary)
    }

    fn binding(&self, preparation_id: Uuid, now: Instant) -> AppResult<RegistryBinding> {
        let mut state = self.inner.state.lock();
        gc_locked(&mut state, now);
        let entry = state
            .entries
            .get(&preparation_id)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        Ok(RegistryBinding {
            preparation_id,
            provider: entry.request.binding.clone(),
            consent_fingerprint: entry.request.consent_fingerprint.clone(),
            risk_flags: entry.request.risk_flags.clone(),
            will_send_image: entry.summary.will_send_image,
            operation_token: entry.operation_token,
        })
    }

    fn authorize(
        &self,
        preparation_id: Uuid,
        expected_consent: &ConsentFingerprint,
        now: Instant,
    ) -> AppResult<Uuid> {
        let mut state = self.inner.state.lock();
        gc_locked(&mut state, now);
        let entry = state
            .entries
            .get_mut(&preparation_id)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        if &entry.request.consent_fingerprint != expected_consent
            || entry.request.risk_flags.is_empty()
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let operation_token = Uuid::new_v4();
        entry.operation_token = Some(operation_token);
        Ok(operation_token)
    }

    fn stage_region_capture(
        &self,
        metadata: RegionCaptureMetadata,
        bytes: OwnedCaptureBytes,
        now: Instant,
    ) -> AppResult<()> {
        let encoded_length = bytes.len();
        let mut state = self.inner.state.lock();
        gc_locked(&mut state, now);
        if state.total_image_bytes.saturating_add(encoded_length) > MAX_TOTAL_IMAGE_BYTES {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let entry = state
            .entries
            .get_mut(&metadata.preparation_id)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        if !entry.summary.will_send_image
            || entry.request.capture.is_some()
            || entry.operation_token != Some(metadata.operation_token)
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let ContentAnchor::Region { region } = &entry.request.anchor else {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        };
        let limits = entry
            .request
            .binding
            .capture_limits
            .ok_or_else(AppError::unsupported_provider_capability)?;
        let capture = validate_and_stage_region_capture(
            ExpectedRegionCapture {
                preparation_id: metadata.preparation_id,
                operation_token: metadata.operation_token,
                book_id: entry.request.binding.book.book_id,
                provider_profile_id: entry.request.binding.profile_id,
                model_id: &entry.request.binding.model_id,
                anchor_content_sha256: &region.content_sha256,
                limits,
            },
            &metadata,
            bytes,
        )?;
        entry.request.capture = Some(capture);
        state.total_image_bytes = state.total_image_bytes.saturating_add(encoded_length);
        Ok(())
    }

    fn consume(
        &self,
        preparation_id: Uuid,
        operation_token: Option<Uuid>,
        now: Instant,
    ) -> AppResult<PreparedLearningRequest> {
        let mut state = self.inner.state.lock();
        gc_locked(&mut state, now);
        let entry = state
            .entries
            .get(&preparation_id)
            .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
        let authorization_required = !entry.request.risk_flags.is_empty();
        if entry.operation_token != operation_token
            || authorization_required != operation_token.is_some()
            || (entry.summary.will_send_image && entry.request.capture.is_none())
        {
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

    pub fn invalidate(&self, request: InvalidateLearningPreparations) -> AppResult<u32> {
        validate_invalidation(&request)?;
        let mut state = self.inner.state.lock();
        gc_locked(&mut state, Instant::now());
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
        gc_locked(&mut state, Instant::now());
        u32::try_from(before.saturating_sub(state.entries.len())).unwrap_or(u32::MAX)
    }

    #[cfg(test)]
    pub(crate) fn garbage_collect_at(&self, now: Instant) -> u32 {
        let mut state = self.inner.state.lock();
        let before = state.entries.len();
        gc_locked(&mut state, now);
        u32::try_from(before.saturating_sub(state.entries.len())).unwrap_or(u32::MAX)
    }

    #[cfg(test)]
    pub(crate) fn stats(&self) -> RegistryStats {
        let state = self.inner.state.lock();
        RegistryStats {
            count: state.entries.len(),
            total_text_bytes: state.total_text_bytes,
            total_image_bytes: state.total_image_bytes,
        }
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
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct RegistryStats {
    pub count: usize,
    pub total_text_bytes: usize,
    pub total_image_bytes: usize,
}

fn gc_locked(state: &mut RegistryState, now: Instant) {
    let expired = state
        .entries
        .iter()
        .filter_map(|(id, entry)| (now >= entry.expires_at).then_some(*id))
        .collect::<Vec<_>>();
    for id in expired {
        state.remove(id);
    }
}

fn unique_id(entries: &BTreeMap<Uuid, RegistryEntry>) -> Uuid {
    loop {
        let candidate = Uuid::new_v4();
        if !entries.contains_key(&candidate) {
            return candidate;
        }
    }
}

fn validate_metadata(metadata: &PrepareLearningRequestMetadata) -> AppResult<()> {
    if metadata.model_id.trim().is_empty()
        || metadata.model_id.chars().count() > MAX_MODEL_ID_CODE_POINTS
        || contains_disallowed_control(&metadata.model_id)
    {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    match (&metadata.content_kind, &metadata.anchor) {
        (LearningContentKind::TextSelection, ContentAnchor::Text { selection }) => {
            validate_quote(
                &selection.quote,
                MAX_SELECTED_TEXT_CODE_POINTS,
                MAX_QUOTE_CONTEXT_CODE_POINTS,
            )?;
            validate_document_locator_metadata(&selection.locator)?;
        }
        (
            LearningContentKind::ReliableTextRegion | LearningContentKind::VisualRegion,
            ContentAnchor::Region { region },
        ) => {
            validate_region_locator_metadata(&region.locator)?;
            if !valid_normalized_rect(&region.rect) || !valid_sha256(&region.content_sha256) {
                return Err(invalid_input());
            }
            if let Some(fallback) = &region.text_fallback {
                validate_quote(
                    fallback,
                    RegionAnchor::MAX_FALLBACK_EXACT_CHARS,
                    RegionAnchor::MAX_FALLBACK_CONTEXT_CHARS,
                )?;
            }
        }
        _ => return Err(AppError::new(AppErrorCode::InvalidInput)),
    }
    Ok(())
}

fn validate_quote(
    quote: &crate::domain::TextQuote,
    max_exact: usize,
    max_context: usize,
) -> AppResult<()> {
    if quote.exact.is_empty()
        || quote.exact.chars().count() > max_exact
        || quote.prefix.chars().count() > max_context
        || quote.suffix.chars().count() > max_context
        || contains_disallowed_control(&quote.exact)
        || contains_disallowed_control(&quote.prefix)
        || contains_disallowed_control(&quote.suffix)
    {
        return Err(invalid_input());
    }
    Ok(())
}

fn validate_document_locator_metadata(locator: &DocumentLocator) -> AppResult<()> {
    match locator {
        DocumentLocator::Pdf {
            start_page,
            end_page,
            rects_by_page,
        } => {
            if *start_page == 0 || start_page > end_page {
                return Err(invalid_input());
            }
            if let Some(pages) = rects_by_page {
                if pages.len() > MAX_PDF_RECT_PAGES {
                    return Err(invalid_input());
                }
                let mut total_rects = 0_usize;
                for (page, rects) in pages {
                    total_rects = total_rects
                        .checked_add(rects.len())
                        .ok_or_else(invalid_input)?;
                    if *page < *start_page
                        || *page > *end_page
                        || rects.len() > MAX_PDF_RECTS_PER_PAGE
                        || total_rects > MAX_PDF_RECTS_TOTAL
                        || rects.iter().any(|rect| !valid_normalized_rect(rect))
                    {
                        return Err(invalid_input());
                    }
                }
            }
        }
        DocumentLocator::Epub { cfi, .. } => {
            if cfi.trim().is_empty()
                || cfi.chars().count() > MAX_EPUB_CFI_CODE_POINTS
                || contains_disallowed_control(cfi)
            {
                return Err(invalid_input());
            }
        }
        DocumentLocator::Docx { .. } => {}
    }
    Ok(())
}

fn validate_region_locator_metadata(locator: &RegionLocator) -> AppResult<()> {
    match locator {
        RegionLocator::Pdf { page } if *page > 0 => Ok(()),
        RegionLocator::Epub { cfi, .. }
            if !cfi.trim().is_empty()
                && cfi.chars().count() <= MAX_EPUB_CFI_CODE_POINTS
                && !contains_disallowed_control(cfi) =>
        {
            Ok(())
        }
        RegionLocator::Docx { .. } => Ok(()),
        _ => Err(invalid_input()),
    }
}

fn valid_normalized_rect(rect: &NormalizedRect) -> bool {
    [rect.x, rect.y, rect.width, rect.height]
        .iter()
        .all(|value| value.is_finite())
        && rect.x >= 0.0
        && rect.y >= 0.0
        && rect.width > 0.0
        && rect.height > 0.0
        && rect.x + rect.width <= 1.0
        && rect.y + rect.height <= 1.0
}

fn validated_selected_text(metadata: &PrepareLearningRequestMetadata) -> AppResult<String> {
    let supplied = metadata
        .selected_text
        .as_deref()
        .filter(|value| !value.is_empty());
    let selected = match (&metadata.content_kind, &metadata.anchor) {
        (LearningContentKind::TextSelection, ContentAnchor::Text { selection }) => {
            let supplied = supplied.ok_or_else(invalid_input)?;
            if supplied != selection.quote.exact {
                return Err(invalid_input());
            }
            supplied.to_owned()
        }
        (LearningContentKind::ReliableTextRegion, ContentAnchor::Region { region }) => {
            let fallback = region.text_fallback.as_ref().ok_or_else(invalid_input)?;
            let supplied = supplied.ok_or_else(invalid_input)?;
            if supplied != fallback.exact {
                return Err(invalid_input());
            }
            supplied.to_owned()
        }
        (LearningContentKind::VisualRegion, ContentAnchor::Region { region }) => {
            match (&region.text_fallback, supplied) {
                (Some(fallback), Some(supplied)) if fallback.exact == supplied => {
                    supplied.to_owned()
                }
                (None, None) => VISUAL_REGION_PLACEHOLDER.to_owned(),
                _ => return Err(invalid_input()),
            }
        }
        _ => return Err(invalid_input()),
    };
    if selected.trim().is_empty()
        || selected.chars().count() > MAX_SELECTED_TEXT_CODE_POINTS
        || contains_disallowed_control(&selected)
    {
        return Err(invalid_input());
    }
    Ok(selected)
}

fn current_question(metadata: &PrepareLearningRequestMetadata) -> AppResult<String> {
    let result = match metadata.action {
        LearningAction::Explain => {
            require_absent(&metadata.question, &metadata.target_language)?;
            "Explain the selected textbook content.".to_owned()
        }
        LearningAction::Example => {
            require_absent(&metadata.question, &metadata.target_language)?;
            "Give a concrete learning example for the selected textbook content.".to_owned()
        }
        LearningAction::Derive => {
            require_absent(&metadata.question, &metadata.target_language)?;
            "Derive the selected textbook content with student-facing steps.".to_owned()
        }
        LearningAction::Translate => {
            if metadata.question.is_some() {
                return Err(invalid_input());
            }
            let target = metadata
                .target_language
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(invalid_input)?;
            if target.chars().count() > MAX_TARGET_LANGUAGE_CODE_POINTS
                || contains_disallowed_control(target)
            {
                return Err(invalid_input());
            }
            format!("Translate the selected textbook content into {target}.")
        }
        LearningAction::Ask => {
            if metadata.target_language.is_some() {
                return Err(invalid_input());
            }
            let question = metadata
                .question
                .as_deref()
                .filter(|value| !value.trim().is_empty())
                .ok_or_else(invalid_input)?;
            if question.chars().count() > MAX_QUESTION_CODE_POINTS
                || contains_disallowed_control(question)
            {
                return Err(invalid_input());
            }
            question.to_owned()
        }
    };
    Ok(result)
}

fn require_absent(left: &Option<String>, right: &Option<String>) -> AppResult<()> {
    if left.is_some() || right.is_some() {
        return Err(invalid_input());
    }
    Ok(())
}

async fn load_binding(
    pool: &SqlitePool,
    capabilities: &ProviderCapabilityRegistry,
    book_id: Uuid,
    section_id: Uuid,
    profile_id: Uuid,
    expected_model_id: &str,
    requires_vision: bool,
) -> AppResult<ProviderBindingSnapshot> {
    let app_settings = settings::get_app_settings(pool).await?;
    let required_default = if requires_vision {
        app_settings.default_vision_profile_id
    } else {
        app_settings.default_learning_profile_id
    };
    if required_default != Some(profile_id) {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let profile = providers::load_provider_profile_metadata(pool, profile_id).await?;
    if profile.id != profile_id || profile.model_id != expected_model_id {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let validated_at = profile
        .validated_at
        .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
    let operation = if requires_vision {
        AiOperation::VisionLearning
    } else {
        AiOperation::TextLearning
    };
    if capabilities.operation_support(&profile.kind, &profile.model_id, operation)
        != CapabilitySupport::Supported
    {
        return Err(AppError::unsupported_provider_capability());
    }
    let provider = capabilities
        .capabilities()
        .iter()
        .find(|candidate| candidate.kind == profile.kind)
        .ok_or_else(AppError::unsupported_provider_capability)?;
    let model = provider
        .models
        .iter()
        .find(|candidate| candidate.id == profile.model_id)
        .ok_or_else(AppError::unsupported_provider_capability)?;
    let capture_limits = if requires_vision {
        Some(RegionCaptureLimits::from_provider(
            model
                .image_limits
                .ok_or_else(AppError::unsupported_provider_capability)?,
        )?)
    } else {
        None
    };
    let context_window_tokens = profile
        .context_window_tokens
        .min(model.context_window_tokens);
    if context_window_tokens == 0 {
        return Err(AppError::new(AppErrorCode::InvalidInput));
    }
    let book = load_book_binding(pool, book_id).await?;
    Ok(ProviderBindingSnapshot {
        book,
        section_id,
        profile_id,
        provider_kind: profile.kind,
        provider_display_name: provider.display_name.clone(),
        profile_display_name: profile.display_name,
        model_id: profile.model_id,
        model_display_name: model.display_name.clone(),
        context_window_tokens,
        default_max_output_tokens: model.default_max_output_tokens,
        validated_at,
        context_mode: app_settings.context_mode,
        requires_vision,
        capture_limits,
    })
}

async fn load_book_binding(pool: &SqlitePool, book_id: Uuid) -> AppResult<BookBindingSnapshot> {
    let row =
        sqlx::query("SELECT format, import_status, sha256, updated_at FROM books WHERE id = ?")
            .bind(book_id.to_string())
            .fetch_optional(pool)
            .await?
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if row.try_get::<String, _>("import_status")? != "ready" {
        return Err(AppError::new(AppErrorCode::BookNotReady));
    }
    let source_sha256 = row
        .try_get::<Option<String>, _>("sha256")?
        .filter(|value| valid_sha256(value))
        .ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
    Ok(BookBindingSnapshot {
        book_id,
        format: row.try_get("format")?,
        source_sha256,
        updated_at: row.try_get("updated_at")?,
    })
}

async fn validate_section_scope(
    pool: &SqlitePool,
    binding: &ProviderBindingSnapshot,
    anchor: &ContentAnchor,
) -> AppResult<()> {
    let locator_json = sqlx::query_scalar::<_, String>(
        "SELECT locator_json FROM sections WHERE id = ? AND book_id = ?",
    )
    .bind(binding.section_id.to_string())
    .bind(binding.book.book_id.to_string())
    .fetch_optional(pool)
    .await?
    .ok_or_else(invalid_input)?;
    let section_locator: DocumentLocator = serde_json::from_str(&locator_json)
        .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
    let valid = match (anchor, &section_locator, binding.book.format.as_str()) {
        (
            ContentAnchor::Text { selection },
            DocumentLocator::Pdf {
                start_page: section_start,
                end_page: section_end,
                ..
            },
            "pdf",
        ) => {
            selection
                .section_id
                .is_none_or(|section_id| section_id == binding.section_id)
                && matches!(
                    &selection.locator,
                    DocumentLocator::Pdf { start_page, end_page, .. }
                        if start_page >= section_start && end_page <= section_end
                )
        }
        (
            ContentAnchor::Region {
                region:
                    RegionAnchor {
                        locator: RegionLocator::Pdf { page },
                        ..
                    },
            },
            DocumentLocator::Pdf {
                start_page,
                end_page,
                ..
            },
            "pdf",
        ) => page >= start_page && page <= end_page,
        (ContentAnchor::Text { selection }, DocumentLocator::Epub { section_id, .. }, "epub") => {
            matches!(
                &selection.locator,
                DocumentLocator::Epub { section_id: anchor_section, .. }
                    if anchor_section == section_id && *anchor_section == binding.section_id
            )
        }
        (
            ContentAnchor::Region {
                region:
                    RegionAnchor {
                        locator: RegionLocator::Epub { section_id, .. },
                        ..
                    },
            },
            DocumentLocator::Epub {
                section_id: section_locator_id,
                ..
            },
            "epub",
        ) => section_id == section_locator_id && *section_id == binding.section_id,
        (ContentAnchor::Text { selection }, DocumentLocator::Docx { .. }, "docx") => {
            if selection
                .section_id
                .is_some_and(|section_id| section_id != binding.section_id)
            {
                false
            } else if let DocumentLocator::Docx {
                start_block_id,
                start_offset,
                end_block_id,
                end_offset,
            } = &selection.locator
            {
                valid_docx_range(
                    pool,
                    binding.book.book_id,
                    binding.section_id,
                    *start_block_id,
                    *start_offset,
                    *end_block_id,
                    *end_offset,
                )
                .await?
            } else {
                false
            }
        }
        (
            ContentAnchor::Region {
                region:
                    RegionAnchor {
                        locator: RegionLocator::Docx { block_id },
                        ..
                    },
            },
            DocumentLocator::Docx { .. },
            "docx",
        ) => {
            sqlx::query_scalar::<_, i64>(
                "SELECT COUNT(*) FROM blocks WHERE id = ? AND book_id = ? AND section_id = ?",
            )
            .bind(block_id.to_string())
            .bind(binding.book.book_id.to_string())
            .bind(binding.section_id.to_string())
            .fetch_one(pool)
            .await?
                == 1
        }
        _ => false,
    };
    if valid { Ok(()) } else { Err(invalid_input()) }
}

async fn valid_docx_range(
    pool: &SqlitePool,
    book_id: Uuid,
    section_id: Uuid,
    start_block_id: Uuid,
    start_offset: u32,
    end_block_id: Uuid,
    end_offset: u32,
) -> AppResult<bool> {
    let start = load_block_bounds(pool, book_id, section_id, start_block_id).await?;
    let end = load_block_bounds(pool, book_id, section_id, end_block_id).await?;
    Ok(matches!(
        (start, end),
        (Some((start_ordinal, start_len)), Some((end_ordinal, end_len)))
            if start_ordinal <= end_ordinal
                && start_offset <= start_len
                && end_offset <= end_len
                && (start_ordinal != end_ordinal || start_offset <= end_offset)
    ))
}

async fn load_block_bounds(
    pool: &SqlitePool,
    book_id: Uuid,
    section_id: Uuid,
    block_id: Uuid,
) -> AppResult<Option<(i64, u32)>> {
    let row = sqlx::query(
        "SELECT ordinal, plain_text FROM blocks WHERE id = ? AND book_id = ? AND section_id = ?",
    )
    .bind(block_id.to_string())
    .bind(book_id.to_string())
    .bind(section_id.to_string())
    .fetch_optional(pool)
    .await?;
    row.map(|row| {
        let ordinal = row.try_get("ordinal")?;
        let text: String = row.try_get("plain_text")?;
        let length = u32::try_from(text.chars().count())
            .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
        Ok((ordinal, length))
    })
    .transpose()
}

fn consent_snapshot(
    profile_id: Uuid,
    consents: Vec<ProviderOperationConsent>,
) -> AppResult<ConsentSnapshot> {
    if consents.len() != 3 {
        return Err(AppError::new(AppErrorCode::DatabaseError));
    }
    let mut image_send = None;
    let mut ai_index = None;
    let mut cost_risk = None;
    let mut fingerprint_material = Zeroizing::new(Vec::with_capacity(256));
    for consent in consents {
        if consent.profile_id != profile_id {
            return Err(invalid_input());
        }
        let (slot, category) = match consent.category {
            ProviderOperationConsentCategory::ImageSend => {
                (&mut image_send, b"image_send".as_slice())
            }
            ProviderOperationConsentCategory::AiIndex => (&mut ai_index, b"ai_index".as_slice()),
            ProviderOperationConsentCategory::CostRisk => (&mut cost_risk, b"cost_risk".as_slice()),
        };
        if slot.replace(consent.decision).is_some() {
            return Err(AppError::new(AppErrorCode::DatabaseError));
        }
        fingerprint_material.extend_from_slice(category);
        fingerprint_material.push(b'|');
        fingerprint_material.extend_from_slice(match consent.decision {
            ProviderOperationConsentDecision::Ask => b"ask",
            ProviderOperationConsentDecision::SkipPrompt => b"skip_prompt",
        });
        fingerprint_material.push(b'|');
        fingerprint_material.extend_from_slice(consent.updated_at.to_rfc3339().as_bytes());
        fingerprint_material.push(b';');
    }
    let _ = ai_index.ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?;
    Ok(ConsentSnapshot {
        fingerprint: ConsentFingerprint(Sha256::digest(fingerprint_material.as_slice()).into()),
        image_send: image_send.ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?,
        cost_risk: cost_risk.ok_or_else(|| AppError::new(AppErrorCode::DatabaseError))?,
    })
}

fn request_text_bytes(request: &PreparedLearningRequest) -> usize {
    let mut total = request.prepared_prompt.system.len();
    for message in &request.prepared_prompt.messages {
        total = total.saturating_add(message.content.len());
    }
    total = total.saturating_add(request.teaching_instruction.instruction.len());
    for segment in &request.packed_context.segments {
        total = total
            .saturating_add(segment.content.len())
            .saturating_add(segment.locator_label.len());
    }
    for citation in &request.packed_context.citations {
        total = total.saturating_add(citation.label.len());
    }
    total.saturating_add(anchor_text_bytes(&request.anchor))
}

fn anchor_text_bytes(anchor: &ContentAnchor) -> usize {
    match anchor {
        ContentAnchor::Text { selection } => selection
            .quote
            .exact
            .len()
            .saturating_add(selection.quote.prefix.len())
            .saturating_add(selection.quote.suffix.len()),
        ContentAnchor::Region { region } => region.text_fallback.as_ref().map_or(0, |quote| {
            quote
                .exact
                .len()
                .saturating_add(quote.prefix.len())
                .saturating_add(quote.suffix.len())
        }),
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
    if valid { Ok(()) } else { Err(invalid_input()) }
}

fn contains_disallowed_control(value: &str) -> bool {
    value
        .chars()
        .any(|code_point| code_point.is_control() && code_point != '\n' && code_point != '\t')
}

fn valid_sha256(value: &str) -> bool {
    value.len() == 64
        && value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
}

fn invalid_input() -> AppError {
    AppError::new(AppErrorCode::InvalidInput)
}
