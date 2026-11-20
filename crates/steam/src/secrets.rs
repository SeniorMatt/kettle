//! Refresh tokens live only in the system keyring (Secret Service:
//! gnome-keyring, KWallet, KeePassXC ...). Never on disk, never logged.

use anyhow::{Context, Result};
use std::sync::Once;

const SERVICE: &str = "steam-rust";

fn init() {
    static ONCE: Once = Once::new();
    ONCE.call_once(|| match zbus_secret_service_keyring_store::Store::new() {
        Ok(store) => keyring_core::set_default_store(store),
        Err(e) => eprintln!("keyring unavailable: {e}"),
    });
}

fn entry(account: &str) -> Result<keyring_core::Entry> {
    init();
    keyring_core::Entry::new(SERVICE, account).context("keyring entry")
}

pub fn save_refresh_token(account: &str, token: &str) -> Result<()> {
    entry(account)?.set_password(token).context("saving token to keyring")
}

/// `Ok(None)` when no token is stored for this account.
pub fn load_refresh_token(account: &str) -> Result<Option<String>> {
    match entry(account)?.get_password() {
        Ok(t) => Ok(Some(t)),
        Err(keyring_core::Error::NoEntry) => Ok(None),
        Err(e) => Err(e).context("reading token from keyring"),
    }
}

pub fn delete_refresh_token(account: &str) -> Result<()> {
    match entry(account)?.delete_credential() {
        Ok(()) | Err(keyring_core::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).context("deleting token from keyring"),
    }
}

#[cfg(test)]
mod tests {
    /// Needs a running Secret Service (gnome-keyring / KWallet): `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn keyring_round_trip() {
        let acct = "__steam_rust_selftest__";
        super::save_refresh_token(acct, "dummy-value").unwrap();
        assert_eq!(super::load_refresh_token(acct).unwrap().as_deref(), Some("dummy-value"));
        super::delete_refresh_token(acct).unwrap();
        assert_eq!(super::load_refresh_token(acct).unwrap(), None);
    }
}
