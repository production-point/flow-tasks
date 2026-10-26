//! Tauri commands for the mail connector (`mail_connector_*`).
//!
//! Errors returned to the webview are stable codes / classes the Settings UI
//! can switch on: `mail-not-running`, `permission-denied`,
//! `script-error: <class>`, or a short human message for validation. The
//! connector key goes into the keychain and is never handed back.

use serde::Serialize;
use tauri::{AppHandle, Manager};

use super::jxa::OsascriptRunner;
use super::runner::{build_status, ConnectorRuntime, ConnectorStatus};
use super::state::{now_ms, resolve_selection, validate_flow_url, MailConnectorState};
use super::{list_accounts, MailAccount};
use crate::commands as keychain;

const UNSUPPORTED: &str = "Mail connector is available on macOS";

/// Apple Mail only exists on macOS; on Windows every command says so and the
/// UI shows the same text.
fn ensure_supported() -> Result<(), String> {
    if cfg!(target_os = "macos") {
        Ok(())
    } else {
        Err(UNSUPPORTED.into())
    }
}

fn connector_state(app: &AppHandle) -> Result<tauri::State<'_, MailConnectorState>, String> {
    app.try_state::<MailConnectorState>()
        .ok_or_else(|| "Mail connector storage unavailable".to_string())
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConfigView {
    pub flow_url: Option<String>,
    pub selected: Vec<MailAccount>,
    pub has_key: bool,
}

/// Read Mail's accounts (and remember them, so `set_selected` can resolve ids
/// without asking Mail again).
#[tauri::command]
pub async fn mail_connector_accounts(app: AppHandle) -> Result<Vec<MailAccount>, String> {
    ensure_supported()?;
    // osascript blocks (and can sit on the first-run Automation prompt), so
    // keep it off the async runtime's worker threads and off the main thread.
    let accounts = tauri::async_runtime::spawn_blocking(|| list_accounts(&OsascriptRunner))
        .await
        .map_err(|_| "Mail connector task failed".to_string())?
        .map_err(|e| e.to_string())?;
    let state = connector_state(&app)?;
    if let Ok(mut last) = state.last_accounts.lock() {
        *last = accounts.clone();
    }
    Ok(accounts)
}

#[tauri::command]
pub async fn mail_connector_set_key(key: String) -> Result<(), String> {
    ensure_supported()?;
    let key = key.trim();
    if key.is_empty() {
        return Err("Enter a Mail connector key".into());
    }
    keychain::set_mail_connector_key(key)
}

#[tauri::command]
pub async fn mail_connector_clear_key() -> Result<(), String> {
    ensure_supported()?;
    keychain::delete_mail_connector_key()
}

#[tauri::command]
pub async fn mail_connector_has_key() -> Result<bool, String> {
    ensure_supported()?;
    Ok(keychain::get_mail_connector_key()?.is_some())
}

/// Store the ticked accounts. Ids must come from the last
/// `mail_connector_accounts` result; id, name, addresses and kind are saved so
/// the runner can register them with FLOW without a Mail call.
#[tauri::command]
pub async fn mail_connector_set_selected(app: AppHandle, ids: Vec<String>) -> Result<(), String> {
    ensure_supported()?;
    let state = connector_state(&app)?;
    let known = state
        .last_accounts
        .lock()
        .map(|a| a.clone())
        .map_err(|_| "mail connector state poisoned".to_string())?;
    state.update_config(|c| {
        c.selected = resolve_selection(&ids, &known, &c.selected)?;
        Ok(())
    })?;
    Ok(())
}

#[tauri::command]
pub async fn mail_connector_get_config(app: AppHandle) -> Result<ConfigView, String> {
    ensure_supported()?;
    let config = connector_state(&app)?.config();
    Ok(ConfigView {
        flow_url: config.flow_url,
        selected: config.selected.into_iter().map(|s| s.account).collect(),
        has_key: keychain::get_mail_connector_key()?.is_some(),
    })
}

#[tauri::command]
pub async fn mail_connector_set_flow_url(app: AppHandle, url: String) -> Result<(), String> {
    ensure_supported()?;
    let url = validate_flow_url(&url)?;
    connector_state(&app)?.update_config(|c| {
        c.flow_url = Some(url);
        Ok(())
    })?;
    Ok(())
}

/// The connector's status for the Settings UI and the title-bar dot. Cheap:
/// in-memory state plus the config; nothing talks to Mail or FLOW.
#[tauri::command]
pub async fn mail_connector_status(app: AppHandle) -> Result<ConnectorStatus, String> {
    ensure_supported()?;
    let config = connector_state(&app)?.config();
    let runtime = app
        .try_state::<ConnectorRuntime>()
        .ok_or_else(|| "Mail connector storage unavailable".to_string())?
        .snapshot();
    let has_key = keychain::get_mail_connector_key()?.is_some();
    Ok(build_status(&config, &runtime, has_key, now_ms()))
}

/// Wake the background loop now ("Connect" / "Run now"). Returns at once;
/// poll `mail_connector_status` for progress. A wake while a run is in flight
/// is coalesced into one follow-up run.
#[tauri::command]
pub async fn mail_connector_run_now(app: AppHandle) -> Result<(), String> {
    ensure_supported()?;
    app.try_state::<ConnectorRuntime>()
        .ok_or_else(|| "Mail connector storage unavailable".to_string())?
        .wake();
    Ok(())
}
