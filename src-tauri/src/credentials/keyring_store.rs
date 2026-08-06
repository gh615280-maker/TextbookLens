use async_trait::async_trait;
use secrecy::{ExposeSecret, SecretString};

use super::CredentialStore;
use crate::errors::{AppError, AppResult};

const SERVICE: &str = "TextbookLens";

#[derive(Clone, Copy, Debug, Default)]
pub struct KeyringCredentialStore;

impl KeyringCredentialStore {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl CredentialStore for KeyringCredentialStore {
    async fn set(&self, key: &str, value: SecretString) -> AppResult<()> {
        let key = key.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            let entry = keyring::Entry::new(SERVICE, &key).map_err(map_backend_error)?;
            entry
                .set_password(value.expose_secret())
                .map_err(map_backend_error)
        })
        .await
        .map_err(|_| AppError::credential_store("credential worker failed"))?
    }

    async fn get(&self, key: &str) -> AppResult<SecretString> {
        let key = key.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            let entry = keyring::Entry::new(SERVICE, &key).map_err(map_backend_error)?;
            entry
                .get_password()
                .map(SecretString::from)
                .map_err(map_backend_error)
        })
        .await
        .map_err(|_| AppError::credential_store("credential worker failed"))?
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        let key = key.to_owned();
        tauri::async_runtime::spawn_blocking(move || {
            let entry = keyring::Entry::new(SERVICE, &key).map_err(map_backend_error)?;
            match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(error) => Err(map_backend_error(error)),
            }
        })
        .await
        .map_err(|_| AppError::credential_store("credential worker failed"))?
    }

    async fn list_textbooklens_keys(&self) -> AppResult<Vec<String>> {
        tauri::async_runtime::spawn_blocking(list_textbooklens_keys)
            .await
            .map_err(|_| AppError::credential_store("credential worker failed"))?
    }
}

#[cfg(windows)]
fn list_textbooklens_keys() -> AppResult<Vec<String>> {
    use std::collections::HashMap;

    use keyring_core::api::CredentialStoreApi as _;

    let store = windows_native_keyring_store::Store::new().map_err(map_backend_error)?;
    let entries = store
        .search(&HashMap::from([("pattern", r"[.]TextbookLens$")]))
        .map_err(map_backend_error)?;
    let mut keys: Vec<String> = Vec::new();
    for entry in entries {
        let Some((service, user)) = entry.get_specifiers() else {
            continue;
        };
        if service == SERVICE && is_textbooklens_logical_key(&user) {
            keys.push(user);
        }
    }
    keys.sort();
    keys.dedup();
    Ok(keys)
}

#[cfg(not(windows))]
fn list_textbooklens_keys() -> AppResult<Vec<String>> {
    Ok(Vec::new())
}

fn is_textbooklens_logical_key(value: &str) -> bool {
    let Some(suffix) = value.strip_prefix("textbooklens/") else {
        return false;
    };
    let id = suffix.strip_prefix("remote-resource/").unwrap_or(suffix);
    uuid::Uuid::parse_str(id).is_ok_and(|parsed| parsed.to_string() == id)
}

fn map_backend_error(error: keyring::Error) -> AppError {
    AppError::credential_store(error)
}
