use anyhow::{bail, Result};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

pub trait KeyringProvider: Send + Sync {
    fn get_secret(&self, service: &str, account: &str) -> Result<Option<String>>;
    fn set_secret(&self, service: &str, account: &str, secret: &str) -> Result<()>;
    fn delete_secret(&self, service: &str, account: &str) -> Result<()>;
    fn is_available(&self) -> bool;
}

/// In-memory mock keyring for testing and environments where no OS daemon is present
#[derive(Clone, Default)]
pub struct MockKeyring {
    store: Arc<Mutex<HashMap<(String, String), String>>>,
}

impl MockKeyring {
    pub fn new() -> Self {
        Self {
            store: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

impl KeyringProvider for MockKeyring {
    fn get_secret(&self, service: &str, account: &str) -> Result<Option<String>> {
        let guard = self.store.lock().unwrap();
        Ok(guard.get(&(service.to_string(), account.to_string())).cloned())
    }

    fn set_secret(&self, service: &str, account: &str, secret: &str) -> Result<()> {
        let mut guard = self.store.lock().unwrap();
        guard.insert((service.to_string(), account.to_string()), secret.to_string());
        Ok(())
    }

    fn delete_secret(&self, service: &str, account: &str) -> Result<()> {
        let mut guard = self.store.lock().unwrap();
        guard.remove(&(service.to_string(), account.to_string()));
        Ok(())
    }

    fn is_available(&self) -> bool {
        true
    }
}

/// Fallback keyring that explicitly signals absence or stores fallback secrets
#[derive(Clone, Default)]
pub struct FallbackKeyring {
    fallback_store: Arc<Mutex<HashMap<(String, String), String>>>,
    strict: bool,
}

impl FallbackKeyring {
    pub fn new(strict: bool) -> Self {
        Self {
            fallback_store: Arc::new(Mutex::new(HashMap::new())),
            strict,
        }
    }
}

impl KeyringProvider for FallbackKeyring {
    fn get_secret(&self, service: &str, account: &str) -> Result<Option<String>> {
        let guard = self.fallback_store.lock().unwrap();
        Ok(guard.get(&(service.to_string(), account.to_string())).cloned())
    }

    fn set_secret(&self, service: &str, account: &str, secret: &str) -> Result<()> {
        if self.strict {
            bail!("OS Keyring daemon unavailable; refusing to store secret in plaintext");
        }
        let mut guard = self.fallback_store.lock().unwrap();
        guard.insert((service.to_string(), account.to_string()), secret.to_string());
        Ok(())
    }

    fn delete_secret(&self, service: &str, account: &str) -> Result<()> {
        let mut guard = self.fallback_store.lock().unwrap();
        guard.remove(&(service.to_string(), account.to_string()));
        Ok(())
    }

    fn is_available(&self) -> bool {
        !self.strict
    }
}
