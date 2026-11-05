//! Keys never enter profile JSON, CLI argv, normal output or coder environments.
pub trait CredentialVault {
    fn put(&self, account: &str, secret: &str) -> Result<(), String>;
    fn get(&self, account: &str) -> Result<String, String>;
    fn delete(&self, account: &str) -> Result<(), String>;
}
pub struct NativeVault;

#[cfg(any(target_os = "macos", windows))]
impl CredentialVault for NativeVault {
    fn put(&self, account: &str, secret: &str) -> Result<(), String> {
        keyring::Entry::new("dev.nova.providers", account)
            .and_then(|entry| entry.set_password(secret))
            .map_err(|_| "native credential store is unavailable or locked".into())
    }
    fn get(&self, account: &str) -> Result<String, String> {
        keyring::Entry::new("dev.nova.providers", account)
            .and_then(|entry| entry.get_password())
            .map_err(|_| "provider key is missing or credential store is locked".into())
    }
    fn delete(&self, account: &str) -> Result<(), String> {
        match keyring::Entry::new("dev.nova.providers", account)
            .and_then(|entry| entry.delete_credential())
        {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err("could not remove provider key from credential store".into()),
        }
    }
}
#[cfg(not(any(target_os = "macos", windows)))]
impl CredentialVault for NativeVault {
    fn put(&self, _: &str, _: &str) -> Result<(), String> {
        Err("native vault is not configured on this platform; use --key-env instead".into())
    }
    fn get(&self, _: &str) -> Result<String, String> {
        Err("native vault is not configured on this platform; use --key-env instead".into())
    }
    fn delete(&self, _: &str) -> Result<(), String> {
        Err("native vault is not configured on this platform".into())
    }
}
