use tauri::State;

use crate::{
    app_state::AppState,
    db::{providers, settings},
    documents::import::ImportService,
    domain::{
        CredentialStatus, ImportStatus, LocalTextQuality, OnboardingStateDto, OnboardingStep,
    },
    errors::AppErrorDto,
};

#[tauri::command]
pub async fn get_onboarding_state(
    state: State<'_, AppState>,
) -> Result<OnboardingStateDto, AppErrorDto> {
    let books = ImportService::with_cancellations(
        state.db.pool().clone(),
        state.paths.clone(),
        state.import_cancellations.clone(),
    )
    .list_books()
    .await
    .map_err(AppErrorDto::from)?;
    let settings = settings::get_app_settings(state.db.pool())
        .await
        .map_err(AppErrorDto::from)?;
    let profiles =
        providers::list_provider_profiles(state.db.pool(), state.credential_store.as_ref())
            .await
            .map_err(AppErrorDto::from)?;
    let selected_book = books
        .iter()
        .find(|book| book.import_status == ImportStatus::Ready)
        .or_else(|| books.first())
        .cloned();
    let has_ready_book = books
        .iter()
        .any(|book| book.import_status == ImportStatus::Ready);
    let connected = |id: Option<uuid::Uuid>| {
        id.and_then(|profile_id| profiles.iter().find(|profile| profile.id == profile_id))
            .is_some_and(|profile| profile.credential_status == CredentialStatus::Available)
    };
    let learning_profile_connected = connected(settings.default_learning_profile_id);
    let vision_profile_connected = settings
        .default_vision_profile_id
        .and_then(|profile_id| profiles.iter().find(|profile| profile.id == profile_id))
        .is_some_and(|profile| {
            profile.credential_status == CredentialStatus::Available
                && state.provider_capabilities.operation_support(
                    &profile.kind,
                    &profile.model_id,
                    crate::domain::AiOperation::VisionLearning,
                ) == crate::domain::CapabilitySupport::Supported
        });
    let local_text_quality = match selected_book.as_ref() {
        Some(book) if book.import_status == ImportStatus::Ready => LocalTextQuality::Ready,
        Some(book)
            if matches!(
                book.import_status,
                ImportStatus::Queued
                    | ImportStatus::Copying
                    | ImportStatus::Parsing
                    | ImportStatus::Indexing
            ) =>
        {
            LocalTextQuality::Pending
        }
        Some(book)
            if book.format == crate::domain::BookFormat::Pdf
                && book.import_status == ImportStatus::Failed
                && book.import_error_code.as_deref() == Some("NO_EXTRACTABLE_TEXT") =>
        {
            LocalTextQuality::VisualSetupRecommended
        }
        _ => LocalTextQuality::Unavailable,
    };
    let step = if !has_ready_book && selected_book.is_none() {
        OnboardingStep::Book
    } else if !learning_profile_connected {
        OnboardingStep::Provider
    } else {
        OnboardingStep::Ready
    };
    Ok(OnboardingStateDto {
        step,
        selected_book,
        has_ready_book,
        learning_profile_connected,
        vision_profile_connected,
        local_text_quality,
        // Completion is derived each bootstrap; never trust the legacy Boolean alone.
        can_skip_onboarding: has_ready_book && learning_profile_connected,
    })
}
