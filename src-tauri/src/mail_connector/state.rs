//! Connector config + read marks, as JSON files in the app data dir.
//!
//! - `mail-connector-config.json`: FLOW URL, selected accounts (id, name,
//!   addresses, kind — stored at selection time so the runner can register
//!   them without asking Mail), and per-account connectionId / sent-today
//!   counts. No key (that's in the keychain), no message content.
//! - `mail-connector-marks.json`: accountId → mailbox path → epoch ms of the
//!   newest message posted. Dates and mailbox names only.
//!
//! Both are written atomically (temp file + rename) so a crash mid-write can't
//! leave a half-written file. A missing or unreadable file loads as empty:
//! for marks that just means re-reading the last 60 days, which FLOW dedupes.

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::MailAccount;

pub const CONFIG_FILE: &str = "mail-connector-config.json";
pub const MARKS_FILE: &str = "mail-connector-marks.json";

/// Each run re-reads 10 minutes before the mark: Mail can surface a message
/// late (sync lag) with a `dateReceived` just before the newest one we saw.
/// FLOW dedupes the overlap by Message-ID.
pub const OVERLAP_MS: i64 = 10 * 60 * 1000;
/// A mailbox with no mark starts 60 days back.
pub const INITIAL_LOOKBACK_MS: i64 = 60 * 24 * 60 * 60 * 1000;
/// FLOW's `POST /accounts` takes at most 20 accounts.
pub const MAX_SELECTED_ACCOUNTS: usize = 20;

/// A selected account plus the runner's per-account bookkeeping.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SelectedAccount {
    #[serde(flatten)]
    pub account: MailAccount,
    /// FLOW's id for this mailbox connection, from the last `POST /accounts`.
    #[serde(default)]
    pub connection_id: Option<String>,
    /// Messages posted today (a count only).
    #[serde(default)]
    pub sent_today: u32,
    /// Local date (`YYYY-MM-DD`) `sent_today` belongs to; it resets when this
    /// isn't today.
    #[serde(default)]
    pub sent_today_date: Option<String>,
}

impl SelectedAccount {
    pub fn new(account: MailAccount) -> Self {
        SelectedAccount { account, connection_id: None, sent_today: 0, sent_today_date: None }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorConfig {
    /// FLOW base URL (validated, no trailing `/`). `None` until the user sets
    /// it — the Settings UI prefills it from the tasks API URL.
    #[serde(default)]
    pub flow_url: Option<String>,
    #[serde(default)]
    pub selected: Vec<SelectedAccount>,
}

/// accountId → mailbox path → epoch ms.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Marks(pub BTreeMap<String, BTreeMap<String, i64>>);

impl Marks {
    pub fn get(&self, account_id: &str, mailbox: &str) -> Option<i64> {
        self.0.get(account_id)?.get(mailbox).copied()
    }

    pub fn set(&mut self, account_id: &str, mailbox: &str, ms: i64) {
        self.0
            .entry(account_id.to_string())
            .or_default()
            .insert(mailbox.to_string(), ms);
    }

    /// Forget every mark of an account, so each of its mailboxes starts
    /// `INITIAL_LOOKBACK_MS` back (used when FLOW's `resumeFrom` is null).
    pub fn clear_account(&mut self, account_id: &str) {
        self.0.remove(account_id);
    }
}

/// Where a read starts: the mark minus the overlap, or 60 days back with no mark.
pub fn read_start(mark: Option<i64>, now_ms: i64) -> i64 {
    match mark {
        Some(m) => m - OVERLAP_MS,
        None => now_ms - INITIAL_LOOKBACK_MS,
    }
}

pub fn now_ms() -> i64 {
    chrono::Utc::now().timestamp_millis()
}

pub fn load_marks(path: &Path) -> Marks {
    load_json(path)
}

pub fn save_marks(path: &Path, marks: &Marks) -> Result<(), String> {
    write_json_atomic(path, marks)
}

pub fn load_config(path: &Path) -> ConnectorConfig {
    load_json(path)
}

pub fn save_config(path: &Path, config: &ConnectorConfig) -> Result<(), String> {
    write_json_atomic(path, config)
}

fn load_json<T: DeserializeOwned + Default>(path: &Path) -> T {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice(&bytes).ok())
        .unwrap_or_default()
}

/// Write to `<file>.tmp` in the same directory, flush, then rename over the
/// target — rename is atomic on the same filesystem (and replaces the target
/// on Windows too).
fn write_json_atomic<T: Serialize>(path: &Path, value: &T) -> Result<(), String> {
    use std::io::Write;

    let fail = |e: std::io::Error| format!("Could not save mail connector state ({:?})", e.kind());
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(fail)?;
    }
    let bytes = serde_json::to_vec_pretty(value)
        .map_err(|_| "Could not save mail connector state (serialise)".to_string())?;
    let mut tmp_name = path.file_name().unwrap_or_default().to_os_string();
    tmp_name.push(".tmp");
    let tmp = path.with_file_name(tmp_name);
    {
        let mut f = std::fs::File::create(&tmp).map_err(fail)?;
        f.write_all(&bytes).map_err(fail)?;
        f.sync_all().map_err(fail)?;
    }
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        fail(e)
    })
}

/// Validate and normalise the FLOW URL: https only (http allowed for
/// `localhost` / `127.0.0.1`, for a local scratch server), no query or
/// fragment, trailing `/` trimmed so paths can be appended.
pub fn validate_flow_url(url: &str) -> Result<String, String> {
    const MSG: &str = "Enter the FLOW address, starting https://";
    let trimmed = url.trim().trim_end_matches('/');
    let parsed = tauri::Url::parse(trimmed).map_err(|_| MSG.to_string())?;
    let host = parsed.host_str().filter(|h| !h.is_empty()).ok_or_else(|| MSG.to_string())?;
    let allowed = match parsed.scheme() {
        "https" => true,
        "http" => host == "localhost" || host == "127.0.0.1",
        _ => false,
    };
    if !allowed {
        return Err(MSG.into());
    }
    if parsed.query().is_some() || parsed.fragment().is_some() || !parsed.username().is_empty() {
        return Err(MSG.into());
    }
    Ok(trimmed.to_string())
}

/// Turn the ticked account ids into the stored selection. Ids are resolved
/// against the account list last read from Mail; an account that stays
/// selected keeps its connectionId and counts.
pub fn resolve_selection(
    ids: &[String],
    known: &[MailAccount],
    existing: &[SelectedAccount],
) -> Result<Vec<SelectedAccount>, String> {
    let mut out: Vec<SelectedAccount> = Vec::new();
    for id in ids {
        if out.iter().any(|s| &s.account.apple_account_id == id) {
            continue;
        }
        let account = known
            .iter()
            .find(|a| &a.apple_account_id == id)
            .ok_or_else(|| "Unknown Mail account: refresh the account list".to_string())?;
        let selected = match existing.iter().find(|s| &s.account.apple_account_id == id) {
            Some(prev) => SelectedAccount { account: account.clone(), ..prev.clone() },
            None => SelectedAccount::new(account.clone()),
        };
        out.push(selected);
    }
    if out.len() > MAX_SELECTED_ACCOUNTS {
        return Err(format!("Select at most {MAX_SELECTED_ACCOUNTS} accounts"));
    }
    Ok(out)
}

/// Tauri-managed connector state: the config (cached in memory behind a lock
/// so the commands and the runner thread can't lose each other's writes) and
/// the account list from the last `mail_connector_accounts` call.
pub struct MailConnectorState {
    dir: PathBuf,
    config: Mutex<ConnectorConfig>,
    pub last_accounts: Mutex<Vec<MailAccount>>,
}

impl MailConnectorState {
    pub fn open(dir: PathBuf) -> Self {
        let config = load_config(&dir.join(CONFIG_FILE));
        MailConnectorState { dir, config: Mutex::new(config), last_accounts: Mutex::new(Vec::new()) }
    }

    pub fn marks_path(&self) -> PathBuf {
        self.dir.join(MARKS_FILE)
    }

    pub fn config(&self) -> ConnectorConfig {
        self.config.lock().map(|c| c.clone()).unwrap_or_default()
    }

    /// Apply `f` to a copy of the config and persist it; the in-memory copy
    /// only changes if `f` succeeds and the write lands.
    pub fn update_config<F>(&self, f: F) -> Result<ConnectorConfig, String>
    where
        F: FnOnce(&mut ConnectorConfig) -> Result<(), String>,
    {
        let mut guard = self.config.lock().map_err(|_| "mail connector state poisoned".to_string())?;
        let mut next = guard.clone();
        f(&mut next)?;
        save_config(&self.dir.join(CONFIG_FILE), &next)?;
        *guard = next.clone();
        Ok(next)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU32, Ordering};

    /// A fresh, empty directory under the system temp dir.
    fn temp_dir() -> PathBuf {
        static N: AtomicU32 = AtomicU32::new(0);
        let d = std::env::temp_dir().join(format!(
            "flow-tasks-mail-connector-test-{}-{}-{}",
            std::process::id(),
            now_ms(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn acct(id: &str) -> MailAccount {
        MailAccount {
            apple_account_id: id.into(),
            name: format!("Account {id}"),
            addresses: vec![format!("{id}@example.com")],
            kind: "imap".into(),
        }
    }

    #[test]
    fn marks_persist_and_the_overlap_is_applied() {
        let dir = temp_dir();
        let path = dir.join(MARKS_FILE);

        // Missing file → empty marks.
        let mut marks = load_marks(&path);
        assert_eq!(marks, Marks::default());

        marks.set("A", "Inbox", 1_761_472_800_000);
        marks.set("A", "Sync Issues/Conflicts", 5);
        marks.set("B", "INBOX", 7);
        marks.set("A", "Inbox", 1_761_472_900_000); // overwrite
        save_marks(&path, &marks).unwrap();

        let loaded = load_marks(&path);
        assert_eq!(loaded, marks);
        assert_eq!(loaded.get("A", "Inbox"), Some(1_761_472_900_000));
        assert_eq!(loaded.get("A", "Archive"), None);
        assert_eq!(loaded.get("C", "Inbox"), None);
        // Atomic write leaves no temp file behind.
        let names: Vec<_> = std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(names, vec![std::ffi::OsString::from(MARKS_FILE)]);

        // Overlap: start 10 min before the mark.
        let now = 1_761_500_000_000;
        assert_eq!(read_start(loaded.get("A", "Inbox"), now), 1_761_472_900_000 - 600_000);
        // No mark: 60 days back.
        assert_eq!(read_start(loaded.get("A", "Archive"), now), now - 60 * 86_400_000);

        // Resetting an account drops only its marks.
        let mut m = loaded.clone();
        m.clear_account("A");
        assert_eq!(m.get("A", "Inbox"), None);
        assert_eq!(m.get("B", "INBOX"), Some(7));

        // A corrupt file loads as empty rather than failing.
        std::fs::write(&path, "{not json").unwrap();
        assert_eq!(load_marks(&path), Marks::default());

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_round_trips_with_bookkeeping_fields() {
        let dir = temp_dir();
        let state = MailConnectorState::open(dir.clone());
        assert_eq!(state.config(), ConnectorConfig::default());

        state
            .update_config(|c| {
                c.flow_url = Some("https://flow.example.com".into());
                let mut s = SelectedAccount::new(acct("A"));
                s.connection_id = Some("conn-1".into());
                s.sent_today = 3;
                c.selected = vec![s];
                Ok(())
            })
            .unwrap();

        let raw: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(dir.join(CONFIG_FILE)).unwrap()).unwrap();
        assert_eq!(raw["flowUrl"], "https://flow.example.com");
        assert_eq!(raw["selected"][0]["id"], "A");
        assert_eq!(raw["selected"][0]["connectionId"], "conn-1");
        assert_eq!(raw["selected"][0]["sentToday"], 3);

        let reopened = MailConnectorState::open(dir.clone());
        assert_eq!(reopened.config(), state.config());

        // A failing update changes nothing, in memory or on disk.
        assert!(reopened.update_config(|c| {
            c.flow_url = None;
            Err("nope".into())
        }).is_err());
        assert_eq!(reopened.config().flow_url.as_deref(), Some("https://flow.example.com"));

        // Older files without the bookkeeping fields still load.
        std::fs::write(
            dir.join(CONFIG_FILE),
            r#"{"selected":[{"id":"A","name":"n","addresses":["a@x"],"kind":"imap"}]}"#,
        )
        .unwrap();
        let c = load_config(&dir.join(CONFIG_FILE));
        assert_eq!(c.flow_url, None);
        assert_eq!(c.selected[0].sent_today, 0);
        assert_eq!(c.selected[0].connection_id, None);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn flow_url_must_be_https_except_for_localhost() {
        assert_eq!(validate_flow_url("https://flow.example.com/").unwrap(), "https://flow.example.com");
        assert_eq!(validate_flow_url("  https://flow.example.com//  ").unwrap(), "https://flow.example.com");
        assert_eq!(validate_flow_url("https://example.com/flow").unwrap(), "https://example.com/flow");
        assert_eq!(validate_flow_url("http://localhost:3000/").unwrap(), "http://localhost:3000");
        assert_eq!(validate_flow_url("http://127.0.0.1:8787").unwrap(), "http://127.0.0.1:8787");

        for bad in [
            "",
            "flow.example.com",
            "http://flow.example.com",
            "http://localhost.evil.com",
            "ftp://flow.example.com",
            "https://",
            "https://flow.example.com/?x=1",
            "https://flow.example.com/#x",
            "javascript:alert(1)",
        ] {
            assert!(validate_flow_url(bad).is_err(), "{bad:?} should be rejected");
        }
    }

    #[test]
    fn selection_resolves_against_known_accounts() {
        let known = vec![acct("A"), acct("B"), acct("C")];
        let mut existing_b = SelectedAccount::new(acct("B"));
        existing_b.connection_id = Some("conn-b".into());
        existing_b.sent_today = 9;

        let sel = resolve_selection(
            &["C".into(), "B".into(), "C".into()],
            &known,
            &[existing_b.clone(), SelectedAccount::new(acct("A"))],
        )
        .unwrap();
        assert_eq!(sel.len(), 2, "deduplicated");
        assert_eq!(sel[0].account, acct("C"));
        assert_eq!(sel[0].connection_id, None);
        assert_eq!(sel[1], existing_b, "kept bookkeeping for a still-selected account");

        // Fresh metadata from Mail replaces the stored copy.
        let mut renamed = acct("B");
        renamed.name = "Renamed".into();
        let sel = resolve_selection(&["B".into()], &[renamed.clone()], &[existing_b.clone()]).unwrap();
        assert_eq!(sel[0].account, renamed);
        assert_eq!(sel[0].connection_id.as_deref(), Some("conn-b"));

        assert!(resolve_selection(&["Z".into()], &known, &[]).is_err());
        assert_eq!(resolve_selection(&[], &known, &[]).unwrap(), vec![]);

        let many: Vec<MailAccount> = (0..21).map(|i| acct(&i.to_string())).collect();
        let ids: Vec<String> = many.iter().map(|a| a.apple_account_id.clone()).collect();
        assert!(resolve_selection(&ids, &many, &[]).is_err());
    }
}
