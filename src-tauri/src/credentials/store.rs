use std::{collections::HashMap, fmt};

use async_trait::async_trait;
use parking_lot::Mutex;
use secrecy::SecretString;

use crate::errors::{AppError, AppResult};

#[async_trait]
pub trait CredentialStore: Send + Sync {
    async fn set(&self, key: &str, value: SecretString) -> AppResult<()>;
    async fn get(&self, key: &str) -> AppResult<SecretString>;
    async fn delete(&self, key: &str) -> AppResult<()>;
}

#[derive(Default)]
struct MemoryState {
    values: HashMap<String, SecretString>,
    fail_next_set: bool,
    fail_next_delete: bool,
}

#[derive(Default)]
pub struct MemoryCredentialStore {
    state: Mutex<MemoryState>,
}

impl MemoryCredentialStore {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn fail_next_set(&self) {
        self.state.lock().fail_next_set = true;
    }

    pub fn fail_next_delete(&self) {
        self.state.lock().fail_next_delete = true;
    }
}

impl fmt::Debug for MemoryCredentialStore {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("MemoryCredentialStore")
            .field("entry_count", &self.state.lock().values.len())
            .finish()
    }
}

#[async_trait]
impl CredentialStore for MemoryCredentialStore {
    async fn set(&self, key: &str, value: SecretString) -> AppResult<()> {
        let mut state = self.state.lock();
        if state.fail_next_set {
            state.fail_next_set = false;
            return Err(AppError::credential_store("simulated set failure"));
        }
        state.values.insert(key.to_owned(), value);
        Ok(())
    }

    async fn get(&self, key: &str) -> AppResult<SecretString> {
        self.state
            .lock()
            .values
            .get(key)
            .cloned()
            .ok_or_else(|| AppError::credential_store("credential is missing"))
    }

    async fn delete(&self, key: &str) -> AppResult<()> {
        let mut state = self.state.lock();
        if state.fail_next_delete {
            state.fail_next_delete = false;
            return Err(AppError::credential_store("simulated delete failure"));
        }
        state.values.remove(key);
        Ok(())
    }
}
