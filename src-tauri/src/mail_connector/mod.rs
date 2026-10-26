//! Mail connector: reads Apple Mail (read-only, via JXA) so new mail can be
//! posted to FLOW's assistant connector API.
//!
//! Everything here is pure logic that compiles on every platform; only
//! `jxa::OsascriptRunner` actually spawns `osascript`, and it's macOS-only.
//! The osascript boundary is the `ScriptRunner` trait so every read path can be
//! unit-tested against a fake — tests never touch the real Mail.
//!
//! Privacy rule for this whole module: message content (subjects, senders,
//! bodies, headers, recipients, attachment names) is never logged, printed or
//! put into an error string. Errors carry error classes only.

// The read / parse / marks API is consumed by the background runner (next
// task); until it lands parts of it are only exercised by tests.
#![allow(dead_code)]

pub mod commands;
pub mod jxa;
pub mod parse;
pub mod state;

use std::fmt;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Deserializer, Serialize};
use serde_json::json;

/// A Mail account as Apple Mail reports it. Serialises as
/// `{ id, name, addresses, kind }` — the shape the webview and the config file
/// both use.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailAccount {
    /// Mail's stable account id (`acct.id()`), used for `accounts.byId(...)`.
    #[serde(rename = "id")]
    pub apple_account_id: String,
    pub name: String,
    #[serde(default, deserialize_with = "string_list")]
    pub addresses: Vec<String>,
    /// `accountType()` — `"iCloud"`, `"imap"`, `"unknown"` (Exchange) …
    pub kind: String,
}

/// One message as the read script returns it. Times are epoch ms.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RawMailMessage {
    #[serde(default)]
    pub message_id: Option<String>,
    pub date_received: i64,
    #[serde(default)]
    pub date_sent: Option<i64>,
    #[serde(default)]
    pub sender: Option<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default, deserialize_with = "string_list")]
    pub to: Vec<String>,
    #[serde(default, deserialize_with = "string_list")]
    pub cc: Vec<String>,
    #[serde(default, deserialize_with = "string_list")]
    pub attachments: Vec<String>,
    #[serde(default, deserialize_with = "string_or_empty")]
    pub headers: String,
    #[serde(default, deserialize_with = "string_or_empty")]
    pub content: String,
}

/// Result of one `read_mailbox_since` chunk.
#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct MailboxRead {
    /// Oldest first.
    pub messages: Vec<RawMailMessage>,
    /// There are further due messages after the last one returned (limit or
    /// time budget hit) — read again from the newest `date_received`.
    #[serde(default)]
    pub more: bool,
    /// Mail quit mid-read. The partial `messages` are still valid.
    #[serde(default)]
    pub not_running: bool,
}

/// Why a script didn't produce a result. Carries error classes only — never
/// stderr text, which can quote message content.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ScriptError {
    /// Apple Mail isn't running. The helper never launches it.
    NotRunning,
    /// macOS Automation permission denied (-1743).
    PermissionDenied,
    /// Anything else, as a content-free class (e.g. `"osascript error -1712"`).
    Other(String),
}

impl fmt::Display for ScriptError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ScriptError::NotRunning => f.write_str("mail-not-running"),
            ScriptError::PermissionDenied => f.write_str("permission-denied"),
            ScriptError::Other(class) => write!(f, "script-error: {class}"),
        }
    }
}

impl std::error::Error for ScriptError {}

/// The osascript boundary. `run_jxa` takes a complete JXA script and returns
/// its stdout (the script's final JSON string).
pub trait ScriptRunner {
    fn run_jxa(&self, script: &str) -> Result<String, ScriptError>;
}

/// Cheap "is Mail running and are we allowed to talk to it" probe. The runner
/// calls this first each tick so it never starts a read it can't finish.
pub fn ping(r: &dyn ScriptRunner) -> Result<(), ScriptError> {
    let _: bool = run(r, jxa::PING_BODY, &json!({}))?;
    Ok(())
}

/// Every Mail account, in Mail's order.
pub fn list_accounts(r: &dyn ScriptRunner) -> Result<Vec<MailAccount>, ScriptError> {
    run(r, jxa::LIST_ACCOUNTS_BODY, &json!({}))
}

/// Every mailbox of an account as a full path (`"Sync Issues/Conflicts"`),
/// nested ones included. Excluded mailboxes are NOT filtered here — use
/// `included_mailboxes`.
pub fn list_mailboxes(r: &dyn ScriptRunner, account_id: &str) -> Result<Vec<String>, ScriptError> {
    run(r, jxa::LIST_MAILBOXES_BODY, &json!({ "account": account_id }))
}

/// Messages in `mailbox` received strictly after `since_ms`, oldest first, at
/// most `limit`, stopping early once `budget_ms` has elapsed inside the script
/// (it always returns at least one message if any are due, so reads progress).
pub fn read_mailbox_since(
    r: &dyn ScriptRunner,
    account_id: &str,
    mailbox: &str,
    since_ms: i64,
    limit: usize,
    budget_ms: u64,
) -> Result<MailboxRead, ScriptError> {
    run(
        r,
        jxa::READ_MAILBOX_BODY,
        &json!({
            "account": account_id,
            "mailbox": mailbox,
            "since": since_ms,
            "limit": limit,
            "budgetMs": budget_ms,
        }),
    )
}

/// Build the script, run it, and unwrap the epilogue's envelope:
/// `{"notRunning":true}` or `{"ok": <main's result>}`.
fn run<T: DeserializeOwned>(
    r: &dyn ScriptRunner,
    body: &str,
    params: &serde_json::Value,
) -> Result<T, ScriptError> {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Envelope<T> {
        #[serde(default)]
        not_running: bool,
        ok: Option<T>,
    }

    let out = r.run_jxa(&jxa::build_script(body, params))?;
    // Deliberately no serde error text in the error: it can quote the payload.
    let env: Envelope<T> = serde_json::from_str(out.trim())
        .map_err(|_| ScriptError::Other("unparseable script output".into()))?;
    if env.not_running {
        return Err(ScriptError::NotRunning);
    }
    env.ok
        .ok_or_else(|| ScriptError::Other("empty script result".into()))
}

/// FLOW's `isExcludedConnectorMailbox` list (lower-cased), plus the iCloud /
/// IMAP names FLOW doesn't list (Junk, Trash, Deleted Messages, Spam, Bin).
/// Stricter than the server is safe: FLOW 400s a batch for an excluded
/// mailbox, so the helper must never post one.
const EXCLUDED_MAILBOXES: &[&str] = &[
    // FLOW's list
    "junk email",
    "deleted items",
    "drafts",
    "outbox",
    "conversation history",
    "sync issues",
    "conflicts",
    "local failures",
    "server failures",
    "journal",
    "tasks",
    "notes",
    "[gmail]/all mail",
    "[gmail]/important",
    "[gmail]/starred",
    "[gmail]/spam",
    "[gmail]/bin",
    "[gmail]/trash",
    "[gmail]/drafts",
    // helper extras
    "junk",
    "trash",
    "deleted messages",
    "spam",
    "bin",
];

/// A mailbox path is excluded if the whole path, or any `/`-prefix of it, is
/// in the list (case-insensitive, trimmed) — so "Sync Issues/Conflicts" and
/// "Deleted Items/Old" are excluded too. Same algorithm as FLOW's.
pub fn is_excluded_mailbox(path: &str) -> bool {
    let listed = |p: &str| {
        let p = p.trim().to_lowercase();
        EXCLUDED_MAILBOXES.contains(&p.as_str())
    };
    listed(path) || path.match_indices('/').any(|(i, _)| listed(&path[..i]))
}

/// The mailboxes the runner may read: `paths` minus the excluded ones, in order.
pub fn included_mailboxes(paths: &[String]) -> Vec<String> {
    paths.iter().filter(|p| !is_excluded_mailbox(p)).cloned().collect()
}

/// Accept a JSON array whose entries may be null / non-strings (Mail sometimes
/// gives `null` for a missing address); keep only the strings, so one odd
/// value can't fail the whole chunk.
fn string_list<'de, D: Deserializer<'de>>(d: D) -> Result<Vec<String>, D::Error> {
    let v = Option::<Vec<serde_json::Value>>::deserialize(d)?;
    Ok(v.unwrap_or_default()
        .into_iter()
        .filter_map(|x| match x {
            serde_json::Value::String(s) => Some(s),
            _ => None,
        })
        .collect())
}

fn string_or_empty<'de, D: Deserializer<'de>>(d: D) -> Result<String, D::Error> {
    Ok(Option::<String>::deserialize(d)?.unwrap_or_default())
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::collections::VecDeque;

    /// Fake osascript: returns queued outputs in order and records every
    /// script it was asked to run.
    pub(crate) struct FakeRunner {
        pub outputs: RefCell<VecDeque<Result<String, ScriptError>>>,
        pub scripts: RefCell<Vec<String>>,
    }

    impl FakeRunner {
        pub fn new(outputs: Vec<Result<String, ScriptError>>) -> Self {
            FakeRunner {
                outputs: RefCell::new(outputs.into()),
                scripts: RefCell::new(Vec::new()),
            }
        }
        pub fn ok(out: &str) -> Self {
            Self::new(vec![Ok(out.to_string())])
        }
    }

    impl ScriptRunner for FakeRunner {
        fn run_jxa(&self, script: &str) -> Result<String, ScriptError> {
            self.scripts.borrow_mut().push(script.to_string());
            self.outputs
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err(ScriptError::Other("fake: no output queued".into())))
        }
    }

    /// Pull the injected `var P = {...};` literal back out of a script.
    pub(crate) fn params_of(script: &str) -> serde_json::Value {
        let line = script
            .lines()
            .find(|l| l.starts_with("var P = "))
            .expect("script has a P line");
        let lit = line.trim_start_matches("var P = ").trim_end_matches(';');
        serde_json::from_str(lit).expect("P is a JSON literal")
    }

    #[test]
    fn skips_when_mail_isnt_running() {
        // RF 1 at the script layer: every entry point reports NotRunning.
        let r = FakeRunner::ok(r#"{"notRunning":true}"#);
        assert_eq!(list_accounts(&r), Err(ScriptError::NotRunning));

        let r = FakeRunner::ok(r#"{"notRunning":true}"#);
        assert_eq!(
            read_mailbox_since(&r, "A", "Inbox", 0, 200, 20_000),
            Err(ScriptError::NotRunning)
        );

        let r = FakeRunner::ok(r#"{"notRunning":true}"#);
        assert_eq!(list_mailboxes(&r, "A"), Err(ScriptError::NotRunning));

        // A failed ping is a single script call: nothing else was attempted.
        let r = FakeRunner::new(vec![
            Ok(r#"{"notRunning":true}"#.into()),
            Ok(r#"{"ok":{"messages":[],"more":false}}"#.into()),
        ]);
        assert_eq!(ping(&r), Err(ScriptError::NotRunning));
        assert_eq!(r.scripts.borrow().len(), 1);
        assert!(!r.scripts.borrow()[0].contains("dateReceived"));
    }

    #[test]
    fn every_script_checks_mail_running_before_touching_it() {
        // The helper never launches Mail: the epilogue guards main().
        for body in [
            jxa::PING_BODY,
            jxa::LIST_ACCOUNTS_BODY,
            jxa::LIST_MAILBOXES_BODY,
            jxa::READ_MAILBOX_BODY,
        ] {
            let s = jxa::build_script(body, &json!({}));
            let guard = s.find("if (!Mail.running())").expect("guard present");
            let call = s.rfind("main(P)").expect("main called");
            assert!(guard < call);
        }
    }

    #[test]
    fn list_accounts_parses_accounts() {
        let r = FakeRunner::ok(
            r#"{"ok":[{"id":"ABC","name":"SPARKS","kind":"unknown","addresses":["ben@sparks.example"]},{"id":"DEF","name":"iCloud","kind":"iCloud","addresses":null}]}"#,
        );
        let accts = list_accounts(&r).unwrap();
        assert_eq!(accts.len(), 2);
        assert_eq!(accts[0].apple_account_id, "ABC");
        assert_eq!(accts[0].addresses, vec!["ben@sparks.example"]);
        assert_eq!(accts[1].kind, "iCloud");
        assert!(accts[1].addresses.is_empty());
    }

    #[test]
    fn read_mailbox_since_passes_params_and_parses_more_and_not_running() {
        let r = FakeRunner::new(vec![
            Ok(r#"{"ok":{"messages":[{"messageId":"a@x","dateReceived":1750000000001,"dateSent":null,"sender":null,"subject":null,"to":["t@x",null],"cc":[],"attachments":[],"headers":"","content":""}],"more":true}}"#.into()),
            Ok(r#"{"ok":{"messages":[],"more":true,"notRunning":true}}"#.into()),
        ]);
        let read = read_mailbox_since(&r, "ACC\"1", "Sync Issues/Conflicts", 1_750_000_000_000, 200, 20_000).unwrap();
        assert!(read.more);
        assert!(!read.not_running);
        assert_eq!(read.messages.len(), 1);
        assert_eq!(read.messages[0].date_received, 1_750_000_000_001);
        assert_eq!(read.messages[0].to, vec!["t@x"]);

        let p = params_of(&r.scripts.borrow()[0]);
        assert_eq!(p["account"], "ACC\"1");
        assert_eq!(p["mailbox"], "Sync Issues/Conflicts");
        assert_eq!(p["since"], 1_750_000_000_000_i64);
        assert_eq!(p["limit"], 200);
        assert_eq!(p["budgetMs"], 20_000);

        let read = read_mailbox_since(&r, "A", "Inbox", 0, 200, 20_000).unwrap();
        assert!(read.not_running, "Mail quit mid-read: partial result kept");
        assert!(read.more);
    }

    #[test]
    fn list_mailboxes_passes_account() {
        let r = FakeRunner::ok(r#"{"ok":["Inbox","Sync Issues/Conflicts"]}"#);
        let mbs = list_mailboxes(&r, "ACC").unwrap();
        assert_eq!(mbs, vec!["Inbox", "Sync Issues/Conflicts"]);
        assert_eq!(params_of(&r.scripts.borrow()[0])["account"], "ACC");
    }

    #[test]
    fn runner_errors_and_garbage_output_pass_through_as_classes() {
        let r = FakeRunner::new(vec![Err(ScriptError::PermissionDenied)]);
        assert_eq!(list_accounts(&r), Err(ScriptError::PermissionDenied));

        let r = FakeRunner::ok("Subject: secret stuff");
        let e = list_accounts(&r).unwrap_err();
        assert_eq!(e, ScriptError::Other("unparseable script output".into()));
        assert!(!e.to_string().contains("secret"));
    }

    #[test]
    fn excluded_mailboxes_are_never_read() {
        // FLOW's names, any case / padding.
        for p in [
            "Junk Email", "deleted items", " Drafts ", "OUTBOX", "Conversation History",
            "Sync Issues", "Conflicts", "Local Failures", "Server Failures", "Journal",
            "Tasks", "Notes", "[Gmail]/All Mail", "[Gmail]/Important", "[Gmail]/Starred",
            "[Gmail]/Spam", "[Gmail]/Bin", "[Gmail]/Trash", "[Gmail]/Drafts",
        ] {
            assert!(is_excluded_mailbox(p), "{p} should be excluded");
        }
        // Helper extras (iCloud / IMAP).
        for p in ["Junk", "Trash", "Deleted Messages", "Spam", "Bin"] {
            assert!(is_excluded_mailbox(p), "{p} should be excluded");
        }
        // Prefix rule: children of an excluded mailbox.
        for p in ["Sync Issues/Conflicts", "Sync Issues/Server Failures", "Deleted Items/Old", "Trash/2024"] {
            assert!(is_excluded_mailbox(p), "{p} should be excluded");
        }
        // Readable mailboxes.
        for p in ["Inbox", "INBOX", "Sent Items", "Sent Messages", "Archive", "Clients/Chauvet", "[Gmail]/Sent Mail", "Notes and Things", "Clients/Junk"] {
            assert!(!is_excluded_mailbox(p), "{p} should be readable");
        }

        // The SPARKS (Exchange) mailbox list measured on Ben's Mac.
        let sparks: Vec<String> = [
            "Conversation History", "Sync Issues", "Sync Issues/Conflicts", "Sync Issues/Local Failures",
            "Sync Issues/Server Failures", "Archive", "Journal", "Tasks", "Notes", "Inbox", "Outbox",
            "Drafts", "Sent Items", "Deleted Items", "Junk Email",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(included_mailboxes(&sparks), vec!["Archive", "Inbox", "Sent Items"]);

        // The iCloud list.
        let icloud: Vec<String> = [
            "Deleted Items", "Junk Email", "Sent Items", "Archive", "Notes", "INBOX", "Drafts",
            "Sent Messages", "Deleted Messages", "Junk",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        assert_eq!(included_mailboxes(&icloud), vec!["Sent Items", "Archive", "INBOX", "Sent Messages"]);
    }
}
