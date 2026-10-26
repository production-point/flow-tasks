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
