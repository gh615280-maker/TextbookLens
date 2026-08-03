mod common;

use std::{
    collections::HashMap,
    fmt,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    time::Duration,
};

use async_trait::async_trait;
use chrono::Utc;
use parking_lot::Mutex;
use secrecy::{ExposeSecret, SecretString};
use textbooklens_lib::{
    ai::registry::ProviderCapabilityRegistry,
    credentials::CredentialStore,
    db::{Database, providers},
    domain::{
        AiOperation, CredentialStatus, ProviderOperationConsentCategory,
        ProviderOperationConsentDecision,
    },
    errors::{AppError, AppErrorCode, AppErrorDto, AppResult},
};
use tokio::sync::{Barrier, Notify};
use uuid::Uuid;

struct BlockingCredentialStore {
    values: Mutex<HashMap<String, SecretString>>,
    block_next_delete: AtomicBool,
    block_next_set: AtomicBool,
    delete_reached: Notify,
    delete_release: Notify,
    set_reached: Notify,
    set_release: Notify,
}

impl BlockingCredentialStore {
    fn new() -> Self {
        Self {
            values: Mutex::new(HashMap::new()),
            block_next_delete: AtomicBool::new(false),
            block_next_set: AtomicBool::new(false),
            delete_reached: Notify::new(),
            delete_release: Notify::new(),
            set_reached: Notify::new(),
            set_release: Notify::new(),
        }
    }

    fn seed(&self, key: &str, value: &str) {
        self.values
            .lock()
            .insert(key.to_owned(), SecretString::from(value));
    }

    fn block_delete(&self) {
        self.block_next_delete.store(true, Ordering::SeqCst);
    }

    fn block_set(&self) {
        self.block_next_set.store(true, Ordering::SeqCst);
    }
}

impl fmt::Debug for BlockingCredentialStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("BlockingCredentialStore")
            .field("entry_count", &self.values.lock().len())
            .finish()
    }
}

#[async_trait]
impl CredentialStore for BlockingCredentialStore {
    async fn set(&self, key: &str, value: SecretString) -> AppResult<()> {
        if self.block_next_set.swap(false, Ordering::SeqCst) {
            self.set_reached.notify_one();
            self.set_release.notified().await;
        }
        self.values.lock().insert(key.to_owned(), value);
        Ok(())
    }

    async fn get(&self, key: &str) -> AppResult<SecretString> {
        self.values
            .lock()
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::credential_store("synthetic credential is missing"))
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        if self.block_next_delete.swap(false, Ordering::SeqCst) {
            self.delete_reached.notify_one();
            self.delete_release.notified().await;
        }
        self.values.lock().remove(key);
        Ok(())
    }
}

#[test]
fn provider_delete_finishes_before_the_onboarding_profile_read() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("library.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let store = Arc::new(BlockingCredentialStore::new());
        let profile_id = Uuid::new_v4();
        insert_profile(database.pool(), profile_id, true).await;
        let key = providers::credential_key(profile_id);
        store.seed(&key, "synthetic-delete-race-value");
        store.block_delete();

        let delete_pool = database.pool().clone();
        let delete_store = store.clone();
        let deletion = tokio::spawn(async move {
            providers::delete_provider_profile(&delete_pool, delete_store, profile_id).await
        });
        store.delete_reached.notified().await;

        let list_pool = database.pool().clone();
        let list_store = store.clone();
        let mut onboarding_read = tokio::spawn(async move {
            providers::list_provider_profiles(&list_pool, list_store.as_ref()).await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut onboarding_read)
                .await
                .is_err(),
            "onboarding must wait instead of observing the delete compensation window"
        );

        store.delete_release.notify_one();
        deletion.await.unwrap().unwrap();
        let profiles = onboarding_read.await.unwrap().unwrap();
        assert!(profiles.is_empty());
        assert!(!format!("{store:?}").contains("synthetic-delete-race-value"));
    });
}

#[test]
fn replacement_compensation_finishes_before_profile_list_returns() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("library.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let store = Arc::new(BlockingCredentialStore::new());
        let profile_id = Uuid::new_v4();
        insert_profile(database.pool(), profile_id, true).await;
        let key = providers::credential_key(profile_id);
        let old_value = "synthetic-replace-race-old";
        let new_value = "synthetic-replace-race-new";
        store.seed(&key, old_value);
        let profile = providers::list_provider_profiles(database.pool(), store.as_ref())
            .await
            .unwrap()
            .remove(0);
        install_update_failure_trigger(database.pool()).await;
        store.block_set();
        let guard = providers::try_provider_mutation(database.pool())
            .await
            .unwrap();

        let replacement_pool = database.pool().clone();
        let replacement_store = store.clone();
        let replacement = tokio::spawn(async move {
            providers::replace_validated_provider_credential(
                &replacement_pool,
                replacement_store,
                guard,
                providers::ReplacementProviderCredential {
                    profile,
                    context_window_tokens: 64_000,
                    credential: SecretString::from(new_value),
                    validated_at: Utc::now(),
                },
            )
            .await
        });
        store.set_reached.notified().await;

        let list_pool = database.pool().clone();
        let list_store = store.clone();
        let mut listing = tokio::spawn(async move {
            providers::list_provider_profiles(&list_pool, list_store.as_ref()).await
        });
        assert!(
            tokio::time::timeout(Duration::from_millis(50), &mut listing)
                .await
                .is_err(),
            "listing must wait until replacement commits or restores the prior credential"
        );

        store.set_release.notify_one();
        let error = replacement.await.unwrap().unwrap_err();
        assert_eq!(error.code, AppErrorCode::DatabaseError);
        let profiles = listing.await.unwrap().unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(profiles[0].context_window_tokens, 32_000);
        assert_eq!(profiles[0].credential_status, CredentialStatus::Available);
        assert_eq!(store.get(&key).await.unwrap().expose_secret(), old_value);

        let public_result = serde_json::to_string(&(AppErrorDto::from(error), profiles)).unwrap();
        for forbidden in [old_value, new_value, "credentialKey", "credential_key"] {
            assert!(!public_result.contains(forbidden), "leaked {forbidden}");
        }
    });
}

#[test]
fn default_switch_and_consent_update_are_serial_local_profile_preferences() {
    let temporary = tempfile::tempdir().unwrap();
    let database = Database::open(temporary.path().join("library.sqlite3")).unwrap();
    tauri::async_runtime::block_on(async {
        let registry = Arc::new(ProviderCapabilityRegistry::load_embedded().unwrap());
        let first_id = Uuid::new_v4();
        let second_id = Uuid::new_v4();
        insert_profile(database.pool(), first_id, true).await;
        insert_profile(database.pool(), second_id, false).await;

        providers::update_provider_operation_consent(
            database.pool(),
            first_id,
            ProviderOperationConsentCategory::ImageSend,
            ProviderOperationConsentDecision::SkipPrompt,
        )
        .await
        .unwrap();
        assert_all_ask(database.pool(), second_id).await;

        let start = Arc::new(Barrier::new(3));
        let default_pool = database.pool().clone();
        let default_registry = registry.clone();
        let default_start = start.clone();
        let default_switch = tokio::spawn(async move {
            default_start.wait().await;
            providers::set_default_provider_profile(
                &default_pool,
                &default_registry,
                AiOperation::TextLearning,
                second_id,
            )
            .await
        });
        let consent_pool = database.pool().clone();
        let consent_start = start.clone();
        let consent_update = tokio::spawn(async move {
            consent_start.wait().await;
            providers::update_provider_operation_consent(
                &consent_pool,
                second_id,
                ProviderOperationConsentCategory::CostRisk,
                ProviderOperationConsentDecision::SkipPrompt,
            )
            .await
        });
        start.wait().await;
        default_switch.await.unwrap().unwrap();
        consent_update.await.unwrap().unwrap();

        let default_id: Option<String> =
            sqlx::query_scalar("SELECT default_learning_profile_id FROM app_settings WHERE id = 1")
                .fetch_one(database.pool())
                .await
                .unwrap();
        assert_eq!(default_id, Some(second_id.to_string()));
        assert_eq!(
            decision(
                database.pool(),
                first_id,
                ProviderOperationConsentCategory::ImageSend
            )
            .await,
            ProviderOperationConsentDecision::SkipPrompt
        );
        assert_eq!(
            decision(
                database.pool(),
                second_id,
                ProviderOperationConsentCategory::CostRisk
            )
            .await,
            ProviderOperationConsentDecision::SkipPrompt
        );

        providers::reset_provider_operation_consents(database.pool(), Some(second_id))
            .await
            .unwrap();
        assert_all_ask(database.pool(), second_id).await;
        assert_eq!(
            decision(
                database.pool(),
                first_id,
                ProviderOperationConsentCategory::ImageSend
            )
            .await,
            ProviderOperationConsentDecision::SkipPrompt,
            "a profile-scoped reset must not copy or reset another profile's choice"
        );

        providers::reset_provider_operation_consents(database.pool(), None)
            .await
            .unwrap();
        assert_all_ask(database.pool(), first_id).await;
        assert_all_ask(database.pool(), second_id).await;

        let book_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM books")
            .fetch_one(database.pool())
            .await
            .unwrap();
        assert_eq!(book_count, 0, "preference commands must not start indexing");
    });
}

async fn insert_profile(pool: &sqlx::SqlitePool, id: Uuid, active: bool) {
    let timestamp = common::utc_timestamp();
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, is_active, created_at, updated_at, validated_at) VALUES (?, 'openai', 'Synthetic provider', 'gpt-5.6', 32000, ?, ?, ?, ?)",
    )
    .bind(id.to_string())
    .bind(active)
    .bind(timestamp)
    .bind(timestamp)
    .bind(timestamp)
    .execute(pool)
    .await
    .unwrap();
    for category in ["image_send", "ai_index", "cost_risk"] {
        sqlx::query(
            "INSERT INTO provider_operation_consents (profile_id, category, decision, updated_at) VALUES (?, ?, 'ask', ?)",
        )
        .bind(id.to_string())
        .bind(category)
        .bind(timestamp)
        .execute(pool)
        .await
        .unwrap();
    }
    if active {
        sqlx::query(
            "UPDATE app_settings SET active_provider_profile_id = ?, default_learning_profile_id = ? WHERE id = 1",
        )
        .bind(id.to_string())
        .bind(id.to_string())
        .execute(pool)
        .await
        .unwrap();
    }
}

async fn install_update_failure_trigger(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "CREATE TRIGGER fail_provider_update BEFORE UPDATE OF context_window_tokens ON provider_profiles BEGIN SELECT RAISE(ABORT, 'synthetic update failure'); END",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn decision(
    pool: &sqlx::SqlitePool,
    profile_id: Uuid,
    category: ProviderOperationConsentCategory,
) -> ProviderOperationConsentDecision {
    providers::list_provider_operation_consents(pool, profile_id)
        .await
        .unwrap()
        .into_iter()
        .find(|consent| consent.category == category)
        .unwrap()
        .decision
}

async fn assert_all_ask(pool: &sqlx::SqlitePool, profile_id: Uuid) {
    let consents = providers::list_provider_operation_consents(pool, profile_id)
        .await
        .unwrap();
    assert_eq!(consents.len(), 3);
    assert!(
        consents
            .iter()
            .all(|consent| consent.decision == ProviderOperationConsentDecision::Ask)
    );
}
