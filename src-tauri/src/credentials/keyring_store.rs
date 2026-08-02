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
}

fn map_backend_error(error: keyring::Error) -> AppError {
    AppError::credential_store(error)
}
