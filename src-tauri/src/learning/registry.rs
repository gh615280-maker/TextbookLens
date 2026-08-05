use std::{
    collections::BTreeMap,
    fmt,
    sync::Arc,
    time::{Duration, Instant},
};

use parking_lot::Mutex;
use tokio_util::sync::CancellationToken;
use uuid::Uuid;
use zeroize::Zeroize;

use crate::{
    domain::{
        Citation, ContentAnchor, LearningAction, LearningRequestEvent, LearningRequestEventPayload,
        LearningRequestSnapshot, LearningRequestStatus, LearningUsage, MAX_LEARNING_OUTPUT_BYTES,
        SafeLearningError,
    },
    errors::{AppError, AppErrorCode, AppResult},
};

pub const LEARNING_REQUEST_TERMINAL_TTL: Duration = Duration::from_secs(10 * 60);
const MAX_ACTIVE_REQUESTS: usize = 64;
const MAX_RETAINED_TERMINAL_REQUESTS: usize = 64;
const MAX_SUBSCRIBERS_PER_REQUEST: usize = 16;

pub type EventSink = Arc<dyn Fn(LearningRequestEvent) -> Result<(), ()> + Send + Sync>;

pub struct NewSelectionRequestContext {
    pub book_id: Uuid,
    pub section_id: Uuid,
    pub anchor: ContentAnchor,
    pub selected_text: Option<String>,
    pub action: LearningAction,
    pub question: String,
}

pub struct FollowupRequestContext {
    pub book_id: Uuid,
    pub conversation_id: Uuid,
    pub expected_next_ordinal: u32,
    pub question: String,
}

pub enum LearningPersistenceTarget {
    NewSelection(NewSelectionRequestContext),
    Followup(FollowupRequestContext),
}

pub struct LearningRequestContext {
    pub target: LearningPersistenceTarget,
    pub provider_profile_id: Uuid,
    pub model_id: String,
    pub available_citations: Vec<Citation>,
}

impl LearningRequestContext {
    pub fn conversation_id(&self) -> Option<Uuid> {
        match &self.target {
            LearningPersistenceTarget::NewSelection(_) => None,
            LearningPersistenceTarget::Followup(target) => Some(target.conversation_id),
        }
    }

    pub fn book_id(&self) -> Uuid {
        match &self.target {
            LearningPersistenceTarget::NewSelection(target) => target.book_id,
            LearningPersistenceTarget::Followup(target) => target.book_id,
        }
    }
}

impl fmt::Debug for LearningRequestContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("LearningRequestContext")
            .field(
                "target",
                &match self.target {
                    LearningPersistenceTarget::NewSelection(_) => "new_selection",
                    LearningPersistenceTarget::Followup(_) => "followup",
                },
            )
            .field("book_id", &"<redacted>")
            .field(
                "conversation_id",
                &self.conversation_id().map(|_| "<redacted>"),
            )
            .field("provider_profile_id", &"<redacted>")
            .field("model_id", &"<redacted>")
            .field("citation_count", &self.available_citations.len())
            .finish()
    }
}

impl Drop for LearningRequestContext {
    fn drop(&mut self) {
        self.model_id.zeroize();
        match &mut self.target {
            LearningPersistenceTarget::NewSelection(target) => {
                target.question.zeroize();
                if let Some(selected_text) = &mut target.selected_text {
                    selected_text.zeroize();
                }
                zeroize_anchor(&mut target.anchor);
            }
            LearningPersistenceTarget::Followup(target) => target.question.zeroize(),
        }
        for citation in &mut self.available_citations {
            citation.id.zeroize();
            citation.label.zeroize();
        }
    }
}

#[derive(Clone)]
pub struct LearningRequestRegistry {
    inner: Arc<RegistryInner>,
}

struct RegistryInner {
    state: Mutex<RegistryState>,
}

#[derive(Default)]
struct RegistryState {
    entries: BTreeMap<Uuid, RegistryEntry>,
    active_conversations: BTreeMap<Uuid, Uuid>,
}

struct RegistryEntry {
    context: Arc<LearningRequestContext>,
    cancel: CancellationToken,
    status: LearningRequestStatus,
    text: String,
    usage: Option<LearningUsage>,
    safe_error: Option<SafeLearningError>,
    conversation_id: Option<Uuid>,
    seq: u32,
    terminal_at: Option<Instant>,
    commit_started: bool,
    commit_irrevocable: bool,
    subscribers: Vec<Subscriber>,
}

impl Drop for RegistryEntry {
    fn drop(&mut self) {
        self.text.zeroize();
        self.cancel.cancel();
    }
}

struct Subscriber {
    id: u32,
    after_seq: u32,
    sink: EventSink,
}

impl Default for LearningRequestRegistry {
    fn default() -> Self {
        Self {
            inner: Arc::new(RegistryInner {
                state: Mutex::new(RegistryState::default()),
            }),
        }
    }
}

impl Drop for RegistryInner {
    fn drop(&mut self) {
        let state = self.state.get_mut();
        for entry in state.entries.values() {
            entry.cancel.cancel();
        }
        state.entries.clear();
        state.active_conversations.clear();
    }
}

impl fmt::Debug for LearningRequestRegistry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let state = self.inner.state.lock();
        formatter
            .debug_struct("LearningRequestRegistry")
            .field("entry_count", &state.entries.len())
            .field(
                "active_conversation_count",
                &state.active_conversations.len(),
            )
            .finish()
    }
}

impl LearningRequestRegistry {
    pub fn create(
        &self,
        context: Arc<LearningRequestContext>,
    ) -> AppResult<LearningRequestSnapshot> {
        self.create_at(context, Instant::now())
    }

    fn create_at(
        &self,
        context: Arc<LearningRequestContext>,
        now: Instant,
    ) -> AppResult<LearningRequestSnapshot> {
        let mut state = self.inner.state.lock();
        gc_locked(&mut state, now);
        let active_request_count = state
            .entries
            .values()
            .filter(|entry| entry.terminal_at.is_none())
            .count();
        if active_request_count >= MAX_ACTIVE_REQUESTS {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        if let Some(conversation_id) = context.conversation_id()
            && state.active_conversations.contains_key(&conversation_id)
        {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        let request_id = unique_request_id(&state.entries);
        let entry = RegistryEntry {
            context,
            cancel: CancellationToken::new(),
            status: LearningRequestStatus::Preparing,
            text: String::new(),
            usage: None,
            safe_error: None,
            conversation_id: None,
            seq: 1,
            terminal_at: None,
            commit_started: false,
            commit_irrevocable: false,
            subscribers: Vec::new(),
        };
        if let Some(conversation_id) = entry.context.conversation_id() {
            state
                .active_conversations
                .insert(conversation_id, request_id);
        }
        let snapshot = snapshot(request_id, &entry)?;
        state.entries.insert(request_id, entry);
        Ok(snapshot)
    }

    pub fn subscribe(
        &self,
        request_id: Uuid,
        after_seq: u32,
        subscriber_id: u32,
        sink: EventSink,
    ) -> AppResult<LearningRequestSnapshot> {
        let mut state = self.inner.state.lock();
        gc_locked(&mut state, Instant::now());
        let entry = state
            .entries
            .get_mut(&request_id)
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
        if after_seq > entry.seq {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        let snapshot = snapshot(request_id, entry)?;
        if entry.terminal_at.is_none() {
            entry
                .subscribers
                .retain(|subscriber| subscriber.id != subscriber_id);
            if entry.subscribers.len() == MAX_SUBSCRIBERS_PER_REQUEST {
                entry.subscribers.remove(0);
            }
            entry.subscribers.push(Subscriber {
                id: subscriber_id,
                after_seq: snapshot.last_seq,
                sink,
            });
        }
        Ok(snapshot)
    }

    pub fn snapshot(&self, request_id: Uuid) -> AppResult<LearningRequestSnapshot> {
        let mut state = self.inner.state.lock();
        gc_locked(&mut state, Instant::now());
        let entry = state
            .entries
            .get(&request_id)
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
        snapshot(request_id, entry)
    }

    pub fn cancellation_token(&self, request_id: Uuid) -> AppResult<CancellationToken> {
        let state = self.inner.state.lock();
        state
            .entries
            .get(&request_id)
            .map(|entry| entry.cancel.clone())
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))
    }

    pub fn commit_context(&self, request_id: Uuid) -> AppResult<Arc<LearningRequestContext>> {
        let state = self.inner.state.lock();
        state
            .entries
            .get(&request_id)
            .map(|entry| entry.context.clone())
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))
    }

    pub fn append_text(&self, request_id: Uuid, delta: String) -> AppResult<()> {
        if delta.is_empty() {
            return Err(AppError::new(AppErrorCode::InvalidInput));
        }
        let mut state = self.inner.state.lock();
        let entry = entry_for_event(&mut state, request_id)?;
        if entry.commit_started {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        if entry.text.len().saturating_add(delta.len()) > MAX_LEARNING_OUTPUT_BYTES {
            entry.cancel.cancel();
            terminal(
                request_id,
                entry,
                LearningRequestStatus::Failed,
                None,
                Some(safe_error(AppErrorCode::ContextTooLarge)),
                LearningRequestEventPayload::Failed {
                    safe_error: safe_error(AppErrorCode::ContextTooLarge),
                },
                Instant::now(),
            )?;
            release_conversation(&mut state, request_id);
            trim_terminal_entries(&mut state);
            return Err(AppError::new(AppErrorCode::ContextTooLarge));
        }
        entry.status = LearningRequestStatus::Streaming;
        entry.text.push_str(&delta);
        emit(
            request_id,
            entry,
            LearningRequestEventPayload::text_delta(delta)
                .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?,
        )
    }

    pub fn record_usage(
        &self,
        request_id: Uuid,
        input_tokens: Option<u64>,
        output_tokens: Option<u64>,
    ) -> AppResult<()> {
        let usage = LearningUsage::new(input_tokens, output_tokens)
            .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?;
        let payload = LearningRequestEventPayload::usage(input_tokens, output_tokens)
            .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?;
        let mut state = self.inner.state.lock();
        let entry = entry_for_event(&mut state, request_id)?;
        if entry.commit_started {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        entry.usage = Some(usage);
        emit(request_id, entry, payload)
    }

    pub fn begin_commit(&self, request_id: Uuid) -> AppResult<bool> {
        let mut state = self.inner.state.lock();
        let entry = state
            .entries
            .get_mut(&request_id)
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
        if entry.terminal_at.is_some() || entry.cancel.is_cancelled() {
            return Ok(false);
        }
        if entry.commit_started || entry.status != LearningRequestStatus::Streaming {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        if entry.text.trim().is_empty() {
            return Err(AppError::new(AppErrorCode::ProviderUnavailable));
        }
        entry.commit_started = true;
        Ok(true)
    }

    pub fn complete(&self, request_id: Uuid, conversation_id: Uuid) -> AppResult<()> {
        let mut state = self.inner.state.lock();
        let entry = state
            .entries
            .get_mut(&request_id)
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
        if !entry.commit_started || !entry.commit_irrevocable || entry.terminal_at.is_some() {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        terminal(
            request_id,
            entry,
            LearningRequestStatus::Completed,
            Some(conversation_id),
            None,
            LearningRequestEventPayload::Completed { conversation_id },
            Instant::now(),
        )?;
        release_conversation(&mut state, request_id);
        trim_terminal_entries(&mut state);
        Ok(())
    }

    pub fn fail(&self, request_id: Uuid, error: &AppError) -> AppResult<()> {
        let mut state = self.inner.state.lock();
        let entry = state
            .entries
            .get_mut(&request_id)
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
        if entry.terminal_at.is_some() {
            return Ok(());
        }
        let safe_error = SafeLearningError::new(error.stable_code())
            .map_err(|_| AppError::new(AppErrorCode::DatabaseError))?;
        terminal(
            request_id,
            entry,
            LearningRequestStatus::Failed,
            None,
            Some(safe_error.clone()),
            LearningRequestEventPayload::Failed { safe_error },
            Instant::now(),
        )?;
        release_conversation(&mut state, request_id);
        trim_terminal_entries(&mut state);
        Ok(())
    }

    pub fn cancel(&self, request_id: Uuid) -> AppResult<()> {
        let mut state = self.inner.state.lock();
        let entry = state
            .entries
            .get_mut(&request_id)
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
        if entry.terminal_at.is_some() || entry.commit_irrevocable {
            return Ok(());
        }
        entry.cancel.cancel();
        terminal(
            request_id,
            entry,
            LearningRequestStatus::Cancelled,
            None,
            None,
            LearningRequestEventPayload::Cancelled,
            Instant::now(),
        )?;
        release_conversation(&mut state, request_id);
        trim_terminal_entries(&mut state);
        Ok(())
    }

    pub fn authorize_database_commit(&self, request_id: Uuid) -> AppResult<bool> {
        let mut state = self.inner.state.lock();
        let entry = state
            .entries
            .get_mut(&request_id)
            .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
        if entry.terminal_at.is_some() || entry.cancel.is_cancelled() {
            return Ok(false);
        }
        if !entry.commit_started || entry.commit_irrevocable {
            return Err(AppError::new(AppErrorCode::RequestConflict));
        }
        entry.commit_irrevocable = true;
        Ok(true)
    }

    pub fn shutdown(&self) {
        let mut state = self.inner.state.lock();
        for entry in state.entries.values() {
            entry.cancel.cancel();
        }
        state.entries.clear();
        state.active_conversations.clear();
    }

    #[cfg(test)]
    pub(crate) fn gc_at(&self, now: Instant) {
        gc_locked(&mut self.inner.state.lock(), now);
    }

    #[cfg(test)]
    pub(crate) fn create_at_for_test(
        &self,
        context: Arc<LearningRequestContext>,
        now: Instant,
    ) -> AppResult<LearningRequestSnapshot> {
        self.create_at(context, now)
    }
}

fn entry_for_event(state: &mut RegistryState, request_id: Uuid) -> AppResult<&mut RegistryEntry> {
    let entry = state
        .entries
        .get_mut(&request_id)
        .ok_or_else(|| AppError::new(AppErrorCode::NotFound))?;
    if entry.terminal_at.is_some() || entry.cancel.is_cancelled() {
        return Err(AppError::new(AppErrorCode::RequestConflict));
    }
    Ok(entry)
}

fn terminal(
    request_id: Uuid,
    entry: &mut RegistryEntry,
    status: LearningRequestStatus,
    conversation_id: Option<Uuid>,
    safe_error: Option<SafeLearningError>,
    payload: LearningRequestEventPayload,
    now: Instant,
) -> AppResult<()> {
    entry.status = status;
    entry.conversation_id = conversation_id;
    entry.safe_error = safe_error;
    entry.terminal_at = Some(now);
    emit(request_id, entry, payload)?;
    entry.subscribers.clear();
    Ok(())
}

fn emit(
    request_id: Uuid,
    entry: &mut RegistryEntry,
    payload: LearningRequestEventPayload,
) -> AppResult<()> {
    entry.seq = entry
        .seq
        .checked_add(1)
        .ok_or_else(|| AppError::new(AppErrorCode::RequestConflict))?;
    let event = LearningRequestEvent::new(request_id, entry.seq, payload)
        .map_err(|_| AppError::new(AppErrorCode::InvalidInput))?;
    entry.subscribers.retain_mut(|subscriber| {
        if event.seq <= subscriber.after_seq {
            return true;
        }
        subscriber.after_seq = event.seq;
        (subscriber.sink)(event.clone()).is_ok()
    });
    Ok(())
}

fn snapshot(request_id: Uuid, entry: &RegistryEntry) -> AppResult<LearningRequestSnapshot> {
    LearningRequestSnapshot::new(
        request_id,
        entry.conversation_id,
        entry.status,
        entry.text.clone(),
        entry.usage.clone(),
        entry.safe_error.clone(),
        entry.seq,
    )
    .map_err(|_| AppError::new(AppErrorCode::DatabaseError))
}

fn release_conversation(state: &mut RegistryState, request_id: Uuid) {
    state
        .active_conversations
        .retain(|_, active_request_id| *active_request_id != request_id);
}

fn gc_locked(state: &mut RegistryState, now: Instant) {
    let expired = state
        .entries
        .iter()
        .filter_map(|(request_id, entry)| {
            entry
                .terminal_at
                .and_then(|terminal_at| now.checked_duration_since(terminal_at))
                .is_some_and(|age| age >= LEARNING_REQUEST_TERMINAL_TTL)
                .then_some(*request_id)
        })
        .collect::<Vec<_>>();
    for request_id in expired {
        state.entries.remove(&request_id);
        release_conversation(state, request_id);
    }
}

fn trim_terminal_entries(state: &mut RegistryState) {
    loop {
        let terminal_count = state
            .entries
            .values()
            .filter(|entry| entry.terminal_at.is_some())
            .count();
        if terminal_count <= MAX_RETAINED_TERMINAL_REQUESTS {
            break;
        }
        let oldest_request_id = state
            .entries
            .iter()
            .filter_map(|(request_id, entry)| {
                entry
                    .terminal_at
                    .map(|terminal_at| (*request_id, terminal_at))
            })
            .min_by_key(|(request_id, terminal_at)| (*terminal_at, *request_id))
            .map(|(request_id, _)| request_id)
            .expect("terminal count proves an oldest entry exists");
        state.entries.remove(&oldest_request_id);
        release_conversation(state, oldest_request_id);
    }
}

fn unique_request_id(entries: &BTreeMap<Uuid, RegistryEntry>) -> Uuid {
    loop {
        let candidate = Uuid::new_v4();
        if !entries.contains_key(&candidate) {
            return candidate;
        }
    }
}

fn safe_error(code: AppErrorCode) -> SafeLearningError {
    SafeLearningError::new(AppError::new(code).stable_code()).expect("stable error codes are valid")
}

fn zeroize_anchor(anchor: &mut ContentAnchor) {
    match anchor {
        ContentAnchor::Text { selection } => {
            selection.quote.exact.zeroize();
            selection.quote.prefix.zeroize();
            selection.quote.suffix.zeroize();
            zeroize_locator(&mut selection.locator);
        }
        ContentAnchor::Region { region } => {
            region.content_sha256.zeroize();
            if let Some(fallback) = &mut region.text_fallback {
                fallback.exact.zeroize();
                fallback.prefix.zeroize();
                fallback.suffix.zeroize();
            }
            if let crate::domain::RegionLocator::Epub { cfi, .. } = &mut region.locator {
                cfi.zeroize();
            }
        }
    }
}

fn zeroize_locator(locator: &mut crate::domain::DocumentLocator) {
    if let crate::domain::DocumentLocator::Epub { cfi, .. } = locator {
        cfi.zeroize();
    }
}

#[cfg(test)]
#[path = "registry_test.rs"]
mod tests;
