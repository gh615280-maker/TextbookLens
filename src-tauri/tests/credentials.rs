mod common;

use secrecy::{ExposeSecret, SecretString};
use textbooklens_lib::{
    credentials::{CredentialStore, MemoryCredentialStore},
    db::{Database, providers},
    errors::AppErrorCode,
};
use uuid::Uuid;

#[cfg(target_os = "windows")]
use textbooklens_lib::credentials::KeyringCredentialStore;

async fn credential_store_contract(store: &MemoryCredentialStore) {
    let key = "textbooklens/contract-profile";
    store
        .set(key, SecretString::from("first-secret"))
        .await
        .unwrap();
    assert_eq!(
        store.get(key).await.unwrap().expose_secret(),
        "first-secret"
    );

    store
        .set(key, SecretString::from("replacement-secret"))
        .await
        .unwrap();
    assert_eq!(
        store.get(key).await.unwrap().expose_secret(),
        "replacement-secret"
    );
    assert!(!format!("{store:?}").contains("replacement-secret"));

    store.delete(key).await.unwrap();
    store.delete(key).await.unwrap();
    assert_eq!(
        store.get(key).await.unwrap_err().code,
        AppErrorCode::CredentialStoreError
    );
}

#[test]
fn memory_credential_store_obeys_the_shared_contract() {
    tauri::async_runtime::block_on(credential_store_contract(&MemoryCredentialStore::new()));
}

#[test]
fn credential_delete_failure_preserves_the_profile_row() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database = Database::open(temp_dir.path().join("library.sqlite3")).unwrap();
    let store = MemoryCredentialStore::new();
    let profile_id = Uuid::new_v4();

    tauri::async_runtime::block_on(async {
        insert_profile(database.pool(), profile_id, true).await;
        let key = providers::credential_key(profile_id);
        store
            .set(&key, SecretString::from("delete-failure-secret"))
            .await
            .unwrap();
        store.fail_next_delete();

        let error = providers::delete_provider_profile(database.pool(), &store, profile_id)
            .await
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::CredentialStoreError);
        assert_eq!(profile_count(database.pool(), profile_id).await, 1);
        assert_eq!(
            store.get(&key).await.unwrap().expose_secret(),
            "delete-failure-secret"
        );
    });
}

#[test]
fn database_delete_failure_restores_the_old_credential() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database = Database::open(temp_dir.path().join("library.sqlite3")).unwrap();
    let store = MemoryCredentialStore::new();
    let profile_id = Uuid::new_v4();

    tauri::async_runtime::block_on(async {
        insert_profile(database.pool(), profile_id, true).await;
        let key = providers::credential_key(profile_id);
        store
            .set(&key, SecretString::from("restored-secret"))
            .await
            .unwrap();
        install_delete_failure_trigger(database.pool()).await;

        let error = providers::delete_provider_profile(database.pool(), &store, profile_id)
            .await
            .unwrap_err();
        assert_eq!(error.code, AppErrorCode::DatabaseError);
        assert_eq!(profile_count(database.pool(), profile_id).await, 1);
        assert_eq!(
            store.get(&key).await.unwrap().expose_secret(),
            "restored-secret"
        );
    });
}

#[test]
fn failed_compensation_surfaces_the_profile_as_missing() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database = Database::open(temp_dir.path().join("library.sqlite3")).unwrap();
    let store = MemoryCredentialStore::new();
    let profile_id = Uuid::new_v4();

    tauri::async_runtime::block_on(async {
        insert_profile(database.pool(), profile_id, true).await;
        let key = providers::credential_key(profile_id);
        store
            .set(&key, SecretString::from("lost-after-delete"))
            .await
            .unwrap();
        install_delete_failure_trigger(database.pool()).await;
        store.fail_next_set();

        providers::delete_provider_profile(database.pool(), &store, profile_id)
            .await
            .unwrap_err();

        let profiles = providers::list_provider_profiles(database.pool(), &store)
            .await
            .unwrap();
        assert_eq!(profiles.len(), 1);
        assert_eq!(
            profiles[0].credential_status,
            providers::CredentialStatus::Missing
        );
        let json = serde_json::to_string(&profiles).unwrap();
        assert!(!json.contains("credentialKey"));
        assert!(!json.contains("lost-after-delete"));
    });
}

#[test]
fn deleting_active_and_final_profiles_reassigns_then_clears_active_profile() {
    let temp_dir = tempfile::tempdir().unwrap();
    let database = Database::open(temp_dir.path().join("library.sqlite3")).unwrap();
    let store = MemoryCredentialStore::new();
    let active_id = Uuid::new_v4();
    let replacement_id = Uuid::new_v4();

    tauri::async_runtime::block_on(async {
        insert_profile(database.pool(), active_id, true).await;
        insert_profile(database.pool(), replacement_id, false).await;
        store
            .set(
                &providers::credential_key(active_id),
                SecretString::from("active-secret"),
            )
            .await
            .unwrap();
        store
            .set(
                &providers::credential_key(replacement_id),
                SecretString::from("replacement-secret"),
            )
            .await
            .unwrap();

        providers::delete_provider_profile(database.pool(), &store, active_id)
            .await
            .unwrap();
        assert_eq!(active_profile(database.pool()).await, Some(replacement_id));

        providers::delete_provider_profile(database.pool(), &store, replacement_id)
            .await
            .unwrap();
        assert_eq!(active_profile(database.pool()).await, None);
    });
}

#[cfg(target_os = "windows")]
#[test]
#[ignore = "manual disposable Windows Credential Manager smoke check"]
fn windows_credential_manager_disposable_round_trip() {
    let store = KeyringCredentialStore::new();
    let key = providers::credential_key(Uuid::new_v4());

    let result = tauri::async_runtime::block_on(async {
        store
            .set(&key, SecretString::from("disposable-smoke-value"))
            .await?;
        let stored = store.get(&key).await?;
        let matches = stored.expose_secret() == "disposable-smoke-value";
        let cleanup = store.delete(&key).await;
        cleanup?;
        Ok::<bool, textbooklens_lib::errors::AppError>(matches)
    });

    assert!(result.unwrap());
}

async fn insert_profile(pool: &sqlx::SqlitePool, profile_id: Uuid, active: bool) {
    let key = providers::credential_key(profile_id);
    sqlx::query(
        "INSERT INTO provider_profiles (id, provider_kind, display_name, model_id, context_window_tokens, credential_key, is_active, created_at, updated_at) VALUES (?, 'openai', 'OpenAI', 'gpt-test', 32000, ?, ?, ?, ?)",
    )
    .bind(profile_id.to_string())
    .bind(&key)
    .bind(active)
    .bind(common::utc_timestamp())
    .bind(common::utc_timestamp())
    .execute(pool)
    .await
    .unwrap();
    if active {
        sqlx::query("UPDATE app_settings SET active_provider_profile_id = ? WHERE id = 1")
            .bind(profile_id.to_string())
            .execute(pool)
            .await
            .unwrap();
    }

    let stored_key: String =
        sqlx::query_scalar("SELECT credential_key FROM provider_profiles WHERE id = ?")
            .bind(profile_id.to_string())
            .fetch_one(pool)
            .await
            .unwrap();
    assert_eq!(stored_key, key);
    assert_ne!(stored_key, "lost-after-delete");
}

async fn install_delete_failure_trigger(pool: &sqlx::SqlitePool) {
    sqlx::query(
        "CREATE TRIGGER fail_provider_delete BEFORE DELETE ON provider_profiles BEGIN SELECT RAISE(ABORT, 'simulated delete failure'); END",
    )
    .execute(pool)
    .await
    .unwrap();
}

async fn profile_count(pool: &sqlx::SqlitePool, profile_id: Uuid) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM provider_profiles WHERE id = ?")
        .bind(profile_id.to_string())
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn active_profile(pool: &sqlx::SqlitePool) -> Option<Uuid> {
    let value: Option<String> =
        sqlx::query_scalar("SELECT active_provider_profile_id FROM app_settings WHERE id = 1")
            .fetch_one(pool)
            .await
            .unwrap();
    value.map(|value| Uuid::parse_str(&value).unwrap())
}
