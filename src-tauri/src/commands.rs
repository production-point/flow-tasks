use std::sync::atomic::Ordering;

use keyring::Entry;

use crate::PinState;

const SERVICE: &str = "flow-tasks";
const ACCOUNT: &str = "api-key";

#[tauri::command]
pub async fn toggle_pin(
    window: tauri::WebviewWindow,
    state: tauri::State<'_, PinState>,
    pinned: bool,
) -> Result<(), String> {
    // Record the flag the macOS hide-on-blur handler reads, then apply the
    // always-on-top half of "pinned".
    state.0.store(pinned, Ordering::Relaxed);
    window.set_always_on_top(pinned).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn store_api_key(key: String) -> Result<(), String> {
    let entry = Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    entry.set_password(&key).map_err(|e| e.to_string())
}

#[tauri::command]
pub async fn get_api_key() -> Result<Option<String>, String> {
    let entry = Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    match entry.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

#[tauri::command]
pub async fn delete_api_key() -> Result<(), String> {
    let entry = Entry::new(SERVICE, ACCOUNT).map_err(|e| e.to_string())?;
    match entry.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

// ── Mail connector key ──────────────────────────────────────────────────────
// A separate keychain entry from the tasks API key (same service, different
// account): it's a different FLOW key type with a different scope, and
// removing one must never remove the other. Read from Rust by the connector
// runner; the webview can only set, clear, or ask whether one exists.

const MAIL_CONNECTOR_ACCOUNT: &str = "flow-tasks-mail-connector";

fn mail_connector_entry() -> Result<Entry, String> {
    Entry::new(SERVICE, MAIL_CONNECTOR_ACCOUNT).map_err(|e| e.to_string())
}

pub fn get_mail_connector_key() -> Result<Option<String>, String> {
    match mail_connector_entry()?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(e.to_string()),
    }
}

/// Whether a Mail connector key is saved, without reading it: only the
/// runner reads the secret. Off macOS there's no connector.
pub fn has_mail_connector_key() -> Result<bool, String> {
    #[cfg(target_os = "macos")]
    return keychain_item_exists(SERVICE, MAIL_CONNECTOR_ACCOUNT);
    #[cfg(not(target_os = "macos"))]
    Ok(false)
}

/// An attributes-only search: the secret is never decrypted, so this can't
/// put up a keychain prompt.
#[cfg(target_os = "macos")]
fn keychain_item_exists(service: &str, account: &str) -> Result<bool, String> {
    use security_framework::item::{ItemClass, ItemSearchOptions, Limit};
    const ERR_SEC_ITEM_NOT_FOUND: i32 = -25300;
    match ItemSearchOptions::new()
        .class(ItemClass::generic_password())
        .service(service)
        .account(account)
        .load_attributes(true)
        .limit(Limit::Max(1))
        .search()
    {
        Ok(found) => Ok(!found.is_empty()),
        Err(e) if e.code() == ERR_SEC_ITEM_NOT_FOUND => Ok(false),
        Err(e) => Err(e.to_string()),
    }
}

pub fn set_mail_connector_key(key: &str) -> Result<(), String> {
    mail_connector_entry()?.set_password(key).map_err(|e| e.to_string())
}

pub fn delete_mail_connector_key() -> Result<(), String> {
    match mail_connector_entry()?.delete_credential() {
        Ok(()) => Ok(()),
        Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;

    /// PR review item 6. Touches the real login keychain, so it's ignored by
    /// default: `cargo test --lib -- --ignored keychain`. The item is made by
    /// `/usr/bin/security`, so this test binary isn't on its access list: if
    /// the check read the secret, macOS would put up a keychain prompt.
    #[test]
    #[ignore]
    fn keychain_item_exists_without_reading_the_secret() {
        const SVC: &str = "flow-tasks-probe-test";
        let security = |args: &[&str]| std::process::Command::new("/usr/bin/security").args(args).output().unwrap();
        security(&["delete-generic-password", "-s", SVC, "-a", "probe"]);
        assert_eq!(keychain_item_exists(SVC, "probe"), Ok(false));
        assert!(security(&["add-generic-password", "-s", SVC, "-a", "probe", "-w", "not-a-secret"]).status.success());
        let found = keychain_item_exists(SVC, "probe");
        security(&["delete-generic-password", "-s", SVC, "-a", "probe"]);
        assert_eq!(found, Ok(true));
        assert_eq!(keychain_item_exists(SVC, "probe"), Ok(false));
    }
}
