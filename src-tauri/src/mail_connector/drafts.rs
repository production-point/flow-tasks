//! Saving FLOW's "Open draft" requests into Apple Mail's Drafts.
//!
//! This is the connector's only write path, and it is deliberately narrow.
//! The draft script may only:
//! - `reply` (no window) on a message it found, or make a new outgoing
//!   message (`visible: false`);
//! - set `content`, `subject`, recipients and `sender`;
//! - `save`.
//!
//! Never send, delete, move, flag, open, activate or close: a test pins the
//! script's verbs to exactly that list. Like the read scripts, it checks
//! `Mail.running()` before touching Mail (it never launches it), and takes its
//! parameters as one JSON literal.
//!
//! Privacy: a request's body, subject and recipients go into the script and
//! nowhere else. Outcomes are codes only (`not-found`, `timeout`, …).

use serde::{Deserialize, Deserializer};
use serde_json::json;

use super::{ScriptError, ScriptRunner};

/// One of FLOW's waiting draft requests (`GET /connector/draft-requests`).
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DraftRequest {
    pub request_id: String,
    /// The original's Message-ID, normalised by FLOW (lower case, no
    /// brackets). Reply mode only.
    #[serde(default)]
    pub internet_message_id: Option<String>,
    /// The mailbox FLOW read the original from (the path this helper posted).
    #[serde(default)]
    pub mailbox: Option<String>,
    pub mode: DraftMode,
    #[serde(default, deserialize_with = "string_list")]
    pub to: Vec<String>,
    #[serde(default, deserialize_with = "string_list")]
    pub cc: Vec<String>,
    #[serde(default)]
    pub subject: Option<String>,
    #[serde(default, deserialize_with = "string_or_empty")]
    pub body_text: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DraftMode {
    Reply,
    New,
    /// A mode this helper doesn't know (a newer FLOW). Reported as a failure
    /// rather than failing the whole response.
    Other,
}

impl<'de> Deserialize<'de> for DraftMode {
    fn deserialize<D: Deserializer<'de>>(d: D) -> Result<Self, D::Error> {
        Ok(match String::deserialize(d)?.as_str() {
            "reply" => DraftMode::Reply,
            "new" => DraftMode::New,
            _ => DraftMode::Other,
        })
    }
}

/// What happened to one request. `Failed` carries a code from
/// `A–Z a–z 0–9 _ . : -`, at most 100 characters: never text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DraftOutcome {
    Saved,
    Failed(String),
}

pub const NOT_FOUND: &str = "not-found";
pub const MAIL_NOT_RUNNING: &str = "mail-not-running";
pub const PERMISSION_DENIED: &str = "permission-denied";
pub const TIMEOUT: &str = "timeout";

impl DraftOutcome {
    /// Mail went away (quit, or Automation was refused) before anything was
    /// saved. Not reported to FLOW: the request stays requested there and is
    /// handed out again next tick.
    pub fn is_mail_unavailable(&self) -> bool {
        matches!(self, DraftOutcome::Failed(c) if c == MAIL_NOT_RUNNING || c == PERMISSION_DENIED)
    }
}

/// Find the original by Message-ID in `P.mailboxes`, in order. Bulk-fetches
/// ids + Message-IDs per mailbox and compares in JS (never `whose(...)`,
/// which froze Mail in the read-script measurements). Both sides are
/// normalised (trimmed, brackets off, lower case): FLOW sends the id
/// normalised and Mail's `messageId` can differ in case.
///
/// Kept separate from the draft body so the real-Mac find-only check runs
/// exactly this function.
pub const FIND_FN: &str = r#"function normId(s) {
  s = String(s === null || s === undefined ? "" : s).trim();
  if (s.charAt(0) === "<") s = s.slice(1);
  if (s.charAt(s.length - 1) === ">") s = s.slice(0, -1);
  return s.trim().toLowerCase();
}
function findMessage(P) {
  var want = normId(P.messageId);
  if (!want) return { searched: 0 };
  var acct = Mail.accounts.byId(P.account), start = Date.now();
  for (var i = 0; i < P.mailboxes.length; i++) {
    if (!Mail.running()) return { notRunning: true };
    if (i > 0 && Date.now() - start > P.budgetMs) return { timeout: true };
    var mb = acct.mailboxes.byName(P.mailboxes[i]), ids, mids;
    try {
      ids = mb.messages.id(); mids = mb.messages.messageId();
      if (ids.length !== mids.length) { ids = mb.messages.id(); mids = mb.messages.messageId(); }
    } catch (e) {
      if (e.errorNumber === -1728) continue;
      throw e;
    }
    if (ids.length !== mids.length) throw new Error("mailbox changed while searching");
    for (var j = 0; j < mids.length; j++) {
      if (normId(mids[j]) === want) return { message: mb.messages.byId(ids[j]), searched: i + 1 };
    }
  }
  return { searched: P.mailboxes.length };
}"#;

/// P: mode, account, sender, mailboxes, messageId, replyToAll, to, cc,
/// subject, bodyText, budgetMs. Returns `{ saved, code? }`.
///
/// Reply: `reply` with no window, the body above Mail's generated quote,
/// `save`. New: an invisible outgoing message with subject + body, the
/// recipients and the account's address as sender, `save`. `saved: true` is
/// returned the moment `save` returns, with no Apple event after it, so a
/// draft that was saved is never reported as anything else.
pub const DRAFT_BODY: &str = r#"function main(P) {
  try {
    var out;
    if (P.mode === "reply") {
      var f = findMessage(P);
      if (f.notRunning) return { saved: false, code: "mail-not-running" };
      if (f.timeout) return { saved: false, code: "timeout" };
      if (!f.message) return { saved: false, code: "not-found" };
      out = f.message.reply({ openingWindow: false, replyToAll: P.replyToAll });
      var quote = "";
      for (var k = 0; k < 10; k++) {
        quote = String(out.content() || "");
        if (quote) break;
        delay(0.2);
      }
      out.content = quote ? P.bodyText + "\n\n" + quote : P.bodyText;
    } else {
      var props = { visible: false, content: P.bodyText };
      if (P.subject !== null) props.subject = P.subject;
      out = Mail.OutgoingMessage(props);
      Mail.outgoingMessages.push(out);
      if (P.sender) out.sender = P.sender;
      P.to.forEach(function (a) { out.toRecipients.push(Mail.ToRecipient({ address: a })); });
      P.cc.forEach(function (a) { out.ccRecipients.push(Mail.CcRecipient({ address: a })); });
    }
    if (!Mail.running()) return { saved: false, code: "mail-not-running" };
    out.save();
    return { saved: true };
  } catch (e) {
    var n = typeof e.errorNumber === "number" ? e.errorNumber : 0;
    if (n === -600 || n === -609) return { saved: false, code: "mail-not-running" };
    if (n === -1743) return { saved: false, code: "permission-denied" };
    return { saved: false, code: "error:" + n };
  }
}"#;

/// The mailboxes to search for a reply's original: the one FLOW named first
/// (matched trimmed and case-insensitively, using Mail's own spelling), then
/// the account's other readable mailboxes in reading order. A message Ben
/// has since moved to Archive is still found.
pub fn search_order(requested: Option<&str>, included: &[String]) -> Vec<String> {
    let want = requested.map(|m| m.trim().to_lowercase()).filter(|m| !m.is_empty());
    let first = want.and_then(|w| included.iter().find(|p| p.trim().to_lowercase() == w));
    first.into_iter().chain(included.iter().filter(|p| Some(*p) != first)).cloned().collect()
}

/// Reply to everyone when FLOW's draft addresses more than one person, as
/// FLOW's own Graph drafts use `createReplyAll` for a message with more than
/// one other address. Mail works out the actual recipients from the original.
pub fn reply_to_all(req: &DraftRequest) -> bool {
    req.to.len() + req.cc.len() > 1
}

/// The complete script for one request. `search` is the reply search order
/// (`search_order`), `sender` the account's address for a new message.
pub fn draft_script(req: &DraftRequest, account: &str, sender: &str, search: &[String], budget_ms: u64) -> String {
    let mode = match req.mode {
        DraftMode::Reply => "reply",
        DraftMode::New => "new",
        DraftMode::Other => "other",
    };
    let body = format!("{FIND_FN}\n{DRAFT_BODY}");
    super::jxa::build_script(
        &body,
        &json!({
            "mode": mode,
            "account": account,
            "sender": sender,
            "mailboxes": search,
            "messageId": req.internet_message_id,
            "replyToAll": reply_to_all(req),
            "to": req.to,
            "cc": req.cc,
            "subject": req.subject,
            "bodyText": req.body_text,
            "budgetMs": budget_ms,
        }),
    )
}

/// Save one request into Mail's Drafts. Never panics, never returns text.
pub fn save_draft(
    r: &dyn ScriptRunner,
    req: &DraftRequest,
    account: &str,
    sender: &str,
    search: &[String],
    budget_ms: u64,
) -> DraftOutcome {
    if req.mode == DraftMode::Other {
        return DraftOutcome::Failed("error:0".into());
    }
    match r.run_jxa(&draft_script(req, account, sender, search, budget_ms)) {
        Ok(out) => parse_outcome(&out),
        Err(e) => failure_code(&e),
    }
}

/// Parse the script's `{"ok":{saved, code?}}` / `{"notRunning":true}`.
pub fn parse_outcome(out: &str) -> DraftOutcome {
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Result {
        saved: bool,
        #[serde(default)]
        code: Option<String>,
    }
    #[derive(Deserialize)]
    #[serde(rename_all = "camelCase")]
    struct Envelope {
        #[serde(default)]
        not_running: bool,
        ok: Option<Result>,
    }
    match serde_json::from_str::<Envelope>(out.trim()) {
        Ok(Envelope { not_running: true, .. }) => DraftOutcome::Failed(MAIL_NOT_RUNNING.into()),
        Ok(Envelope { ok: Some(Result { saved: true, .. }), .. }) => DraftOutcome::Saved,
        Ok(Envelope { ok: Some(Result { code, .. }), .. }) => {
            DraftOutcome::Failed(code.filter(|c| is_valid_code(c)).unwrap_or_else(|| "error:0".into()))
        }
        _ => DraftOutcome::Failed("error:0".into()),
    }
}

/// An osascript failure as a result code.
pub fn failure_code(e: &ScriptError) -> DraftOutcome {
    let code = match e {
        ScriptError::NotRunning => MAIL_NOT_RUNNING.to_string(),
        ScriptError::PermissionDenied => PERMISSION_DENIED.to_string(),
        ScriptError::Other(class) if class == "osascript timeout" => TIMEOUT.to_string(),
        ScriptError::Other(class) => match class.strip_prefix("osascript error ") {
            Some(n) if n.parse::<i64>().is_ok() => format!("error:{n}"),
            _ => "error:0".to_string(),
        },
    };
    DraftOutcome::Failed(code)
}

/// FLOW's rule for a result code: 1–100 of `A–Z a–z 0–9 _ . : -`.
pub fn is_valid_code(c: &str) -> bool {
    !c.is_empty() && c.len() <= 100 && c.chars().all(|ch| ch.is_ascii_alphanumeric() || "_.:-".contains(ch))
}

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
mod tests {
    use super::*;
    use crate::mail_connector::jxa;
    use crate::mail_connector::tests::{params_of, FakeRunner};
    use std::collections::BTreeSet;

    fn req(mode: &str) -> DraftRequest {
        serde_json::from_value(json!({
            "requestId": "r1", "internetMessageId": "abc@x.com", "mailbox": "Inbox", "mode": mode,
            "to": ["a@x.com"], "cc": [], "subject": "Re: hi", "bodyText": "Thanks"
        }))
        .unwrap()
    }

    #[test]
    fn requests_parse_from_flows_json() {
        let r: Vec<DraftRequest> = serde_json::from_value(json!([
            { "requestId": "r1", "internetMessageId": "abc@x.com", "mailbox": "Inbox", "mode": "reply",
              "to": ["a@x.com", null], "cc": ["c@x.com"], "subject": "Re: s", "bodyText": "B" },
            { "requestId": "r2", "internetMessageId": null, "mailbox": null, "mode": "new",
              "to": [], "cc": [], "subject": null, "bodyText": "B2" },
            { "requestId": "r3", "mode": "forward" }
        ]))
        .unwrap();
        assert_eq!(r[0].mode, DraftMode::Reply);
        assert_eq!(r[0].to, vec!["a@x.com"]);
        assert_eq!(r[0].cc, vec!["c@x.com"]);
        assert_eq!(r[1].mode, DraftMode::New);
        assert_eq!(r[1].subject, None);
        assert_eq!(r[2].mode, DraftMode::Other);
        assert_eq!(r[2].body_text, "");
        // An unknown mode is a failure code, and no script runs.
        let fake = FakeRunner::new(vec![]);
        assert_eq!(save_draft(&fake, &r[2], "A", "b@x.com", &[], 20_000), DraftOutcome::Failed("error:0".into()));
        assert!(fake.scripts.borrow().is_empty());
    }

    #[test]
    fn parameters_are_one_json_literal() {
        let mut q = req("new");
        q.body_text = "He said \"hi\"\u{2028}line\u{2029}para\n'); Mail.quit(); ('".into();
        q.subject = Some("Quote \" and \\ backslash".into());
        let s = draft_script(&q, "ACC\"1", "b@x.com", &["Inbox".to_string()], 20_000);
        assert!(!s.contains('\u{2028}') && !s.contains('\u{2029}'));
        let p = params_of(&s);
        assert_eq!(p["bodyText"], q.body_text);
        assert_eq!(p["subject"], "Quote \" and \\ backslash");
        assert_eq!(p["account"], "ACC\"1");
        assert_eq!(p["mode"], "new");
        assert_eq!(p["sender"], "b@x.com");
        assert_eq!(p["mailboxes"], json!(["Inbox"]));
        assert_eq!(p["budgetMs"], 20_000);
        // The content never lands outside the P literal.
        assert_eq!(s.matches("Mail.quit").count(), 1);
    }

    /// Every `.name(` call and every `.name =` assignment in a script.
    fn calls_and_assignments(src: &str) -> (BTreeSet<String>, BTreeSet<String>) {
        let b = src.as_bytes();
        let (mut calls, mut sets) = (BTreeSet::new(), BTreeSet::new());
        let mut i = 0;
        while i < b.len() {
            if b[i] == b'.' && i + 1 < b.len() && (b[i + 1].is_ascii_alphabetic() || b[i + 1] == b'_') {
                let start = i + 1;
                let mut j = start;
                while j < b.len() && (b[j].is_ascii_alphanumeric() || b[j] == b'_') {
                    j += 1;
                }
                let name = &src[start..j];
                let mut k = j;
                while k < b.len() && b[k] == b' ' {
                    k += 1;
                }
                if k < b.len() && b[k] == b'(' {
                    calls.insert(name.to_string());
                } else if k + 1 < b.len() && b[k] == b'=' && b[k + 1] != b'=' {
                    sets.insert(name.to_string());
                }
                i = j;
            } else {
                i += 1;
            }
        }
        (calls, sets)
    }

    #[test]
    fn the_draft_script_can_only_reply_make_set_and_save() {
        // Spec §9 / plan Global Constraints: the write surface is exactly
        // `reply` or make-new-outgoing-message, set content / subject /
        // recipients / sender, then `save`. Never send, delete, move, flag,
        // open, activate or close.
        let script = format!("{FIND_FN}\n{DRAFT_BODY}");
        let (calls, sets) = calls_and_assignments(&script);

        // Reads (no side effects in Mail) and plain JS.
        let reads: BTreeSet<&str> = [
            "running", "byId", "byName", "id", "messageId", "content", "now", "trim", "charAt", "slice",
            "toLowerCase", "forEach",
        ]
        .into_iter()
        .collect();
        // The only verbs that change anything in Mail.
        let writes: BTreeSet<&str> = ["reply", "OutgoingMessage", "push", "ToRecipient", "CcRecipient", "save"].into_iter().collect();
        let unexpected: Vec<&String> = calls.iter().filter(|c| !reads.contains(c.as_str()) && !writes.contains(c.as_str())).collect();
        assert!(unexpected.is_empty(), "calls outside the allowlist: {unexpected:?}");
        for w in &writes {
            assert!(calls.contains(*w), "{w} is part of the draft path");
        }
        // `push` only ever adds the new outgoing message and its recipients.
        let pushes: Vec<_> = script.match_indices(".push(").map(|(i, _)| &script[script[..i].rfind(|c: char| c == ' ' || c == '(').unwrap() + 1..i]).collect();
        assert_eq!(pushes, vec!["Mail.outgoingMessages", "out.toRecipients", "out.ccRecipients"]);
        assert!(script.contains("Mail.OutgoingMessage(props)") && script.contains("visible: false"));
        assert!(script.contains("openingWindow: false"));

        let allowed_sets: BTreeSet<String> = ["content", "subject", "sender"].iter().map(|s| s.to_string()).collect();
        // `props.subject =` builds the constructor's properties (a JS object).
        let sets: BTreeSet<String> = sets.into_iter().collect();
        assert!(sets.is_subset(&allowed_sets), "assignments outside the allowlist: {sets:?}");

        let lower = script.to_lowercase();
        for banned in [".send(", "send(", ".delete(", ".move(", "flag", ".open(", "activate", ".close(", "mail.make(", "whose(", "readstatus", ".mailbox =", "visible: true", "openingwindow: true"] {
            assert!(!lower.contains(banned), "draft script must not contain {banned}");
        }
    }

    #[test]
    fn the_read_scripts_still_have_no_write_verbs() {
        for body in [jxa::PING_BODY, jxa::LIST_ACCOUNTS_BODY, jxa::LIST_MAILBOXES_BODY, jxa::READ_MAILBOX_BODY] {
            let (calls, sets) = calls_and_assignments(body);
            // (`push` here is JS's Array.push on the script's own arrays.)
            for verb in ["reply", "save", "send", "delete", "move", "open", "activate", "close", "make", "OutgoingMessage", "ToRecipient", "CcRecipient"] {
                assert!(!calls.contains(verb), "read script calls {verb}");
            }
            assert!(sets.is_empty(), "read script assigns {sets:?}");
            for verb in [".delete(", ".move(", "Mail.make(", ".save(", ".send(", ".reply(", "flaggedStatus", "readStatus", "whose(", "outgoingMessages"] {
                assert!(!body.contains(verb), "read script must not use {verb}");
            }
        }
    }

    #[test]
    fn the_draft_script_never_launches_mail() {
        let s = draft_script(&req("reply"), "A", "b@x.com", &[], 20_000);
        let guard = s.find("if (!Mail.running())").expect("guard present");
        assert!(guard < s.rfind("main(P)").unwrap());
    }

    #[test]
    fn outcomes_parse_to_codes_only() {
        assert_eq!(parse_outcome(r#"{"ok":{"saved":true}}"#), DraftOutcome::Saved);
        assert_eq!(parse_outcome(r#"{"ok":{"saved":false,"code":"not-found"}}"#), DraftOutcome::Failed("not-found".into()));
        assert_eq!(parse_outcome(r#"{"ok":{"saved":false,"code":"error:-1712"}}"#), DraftOutcome::Failed("error:-1712".into()));
        assert_eq!(parse_outcome(r#"{"notRunning":true}"#), DraftOutcome::Failed("mail-not-running".into()));
        // Anything that isn't a clean code never reaches FLOW.
        assert_eq!(parse_outcome(r#"{"ok":{"saved":false,"code":"Subject: Acme invoice"}}"#), DraftOutcome::Failed("error:0".into()));
        assert_eq!(parse_outcome(&format!(r#"{{"ok":{{"saved":false,"code":"{}"}}}}"#, "x".repeat(101))), DraftOutcome::Failed("error:0".into()));
        assert_eq!(parse_outcome(r#"{"ok":{"saved":false}}"#), DraftOutcome::Failed("error:0".into()));
        assert_eq!(parse_outcome("Dear Ben, …"), DraftOutcome::Failed("error:0".into()));

        assert_eq!(failure_code(&ScriptError::NotRunning), DraftOutcome::Failed("mail-not-running".into()));
        assert_eq!(failure_code(&ScriptError::PermissionDenied), DraftOutcome::Failed("permission-denied".into()));
        assert_eq!(failure_code(&ScriptError::Other("osascript timeout".into())), DraftOutcome::Failed("timeout".into()));
        assert_eq!(failure_code(&ScriptError::Other("osascript error -1712".into())), DraftOutcome::Failed("error:-1712".into()));
        assert_eq!(failure_code(&ScriptError::Other("osascript failed".into())), DraftOutcome::Failed("error:0".into()));

        assert!(DraftOutcome::Failed("mail-not-running".into()).is_mail_unavailable());
        assert!(DraftOutcome::Failed("permission-denied".into()).is_mail_unavailable());
        assert!(!DraftOutcome::Failed("not-found".into()).is_mail_unavailable());
        assert!(!DraftOutcome::Saved.is_mail_unavailable());
        for c in ["not-found", "mail-not-running", "permission-denied", "timeout", "error:-1712", "error:0"] {
            assert!(is_valid_code(c));
        }
    }

    #[test]
    fn a_moved_original_is_searched_for_in_the_other_mailboxes() {
        // Review Focus 1: FLOW named Inbox, Ben has since archived it.
        let included = vec!["INBOX".to_string(), "Sent Items".to_string(), "Archive".to_string(), "Clients/Acme".to_string()];
        assert_eq!(search_order(Some(" inbox "), &included), vec!["INBOX", "Sent Items", "Archive", "Clients/Acme"]);
        assert_eq!(search_order(Some("Archive"), &included), vec!["Archive", "INBOX", "Sent Items", "Clients/Acme"]);
        // A mailbox that's gone (or excluded) is just skipped.
        assert_eq!(search_order(Some("Old"), &included), included);
        assert_eq!(search_order(None, &included), included);

        // The search order reaches the script as given.
        let fake = FakeRunner::ok(r#"{"ok":{"saved":true}}"#);
        let order = search_order(Some("Archive"), &included);
        assert_eq!(save_draft(&fake, &req("reply"), "A", "b@x.com", &order, 20_000), DraftOutcome::Saved);
        let p = params_of(&fake.scripts.borrow()[0]);
        assert_eq!(p["mailboxes"], json!(["Archive", "INBOX", "Sent Items", "Clients/Acme"]));
        assert_eq!(p["messageId"], "abc@x.com");
        assert_eq!(p["replyToAll"], false);

        // And the real script walks it: found in the third mailbox.
        let runs = run_draft_in_node(
            &json!({ "INBOX": [["m1@x", 1]], "Sent Items": [], "Archive": [["other@x", 2], ["ABC@x.com", 3]] }),
            &json!([{ "mode": "reply", "account": "A", "sender": "b@x.com", "mailboxes": ["INBOX", "Sent Items", "Archive"],
                      "messageId": "abc@x.com", "replyToAll": false, "to": [], "cc": [], "subject": null,
                      "bodyText": "Thanks", "budgetMs": 20000 }]),
            &json!({}),
        );
        assert_eq!(runs[0]["result"], json!({ "saved": true }));
        assert_eq!(runs[0]["log"], json!(["reply 3 all=false", "set content", "save"]));
        assert_eq!(runs[0]["content"], "Thanks\n\n> quoted 3");

        // Not anywhere: not-found, nothing written.
        let runs = run_draft_in_node(
            &json!({ "INBOX": [["m1@x", 1]], "Archive": [] }),
            &json!([{ "mode": "reply", "account": "A", "sender": "", "mailboxes": ["INBOX", "Archive"],
                      "messageId": "abc@x.com", "replyToAll": false, "to": [], "cc": [], "subject": null,
                      "bodyText": "Thanks", "budgetMs": 20000 }]),
            &json!({}),
        );
        assert_eq!(runs[0]["result"], json!({ "saved": false, "code": "not-found" }));
        assert_eq!(runs[0]["log"], json!([]));
    }

    #[test]
    fn message_ids_match_whatever_the_case_and_brackets() {
        // Review Focus 2: FLOW sends `abc@x.com`; Mail may say `<ABC@X.Com>`.
        for mail_id in ["<ABC@X.Com>", "abc@x.com", " ABC@x.COM ", "<abc@x.com>"] {
            let runs = run_draft_in_node(
                &json!({ "Inbox": [[mail_id, 7]] }),
                &json!([{ "mode": "reply", "account": "A", "sender": "", "mailboxes": ["Inbox"],
                          "messageId": "abc@x.com", "replyToAll": true, "to": [], "cc": [], "subject": null,
                          "bodyText": "B", "budgetMs": 20000 }]),
                &json!({}),
            );
            assert_eq!(runs[0]["result"], json!({ "saved": true }), "{mail_id}");
            assert_eq!(runs[0]["log"][0], "reply 7 all=true");
        }
        // A different id that merely contains it is not a match.
        let runs = run_draft_in_node(
            &json!({ "Inbox": [["xabc@x.com", 7], ["abc@x.com.au", 8]] }),
            &json!([{ "mode": "reply", "account": "A", "sender": "", "mailboxes": ["Inbox"],
                      "messageId": "abc@x.com", "replyToAll": false, "to": [], "cc": [], "subject": null,
                      "bodyText": "B", "budgetMs": 20000 }]),
            &json!({}),
        );
        assert_eq!(runs[0]["result"]["code"], "not-found");
        // FLOW's own normalisation is undone harmlessly if it ever sends brackets.
        assert_eq!(reply_to_all(&req("reply")), false);
        let mut many = req("reply");
        many.cc = vec!["c@x.com".into()];
        assert!(reply_to_all(&many));
    }

    #[test]
    fn a_new_message_gets_its_recipients_sender_and_is_saved_invisibly() {
        let runs = run_draft_in_node(
            &json!({}),
            &json!([
                { "mode": "new", "account": "A", "sender": "ben@sparks.example", "mailboxes": [],
                  "messageId": null, "replyToAll": false, "to": ["a@x.com"], "cc": ["c@x.com"], "subject": "Hello",
                  "bodyText": "Body", "budgetMs": 20000 },
                { "mode": "new", "account": "A", "sender": "ben@sparks.example", "mailboxes": [],
                  "messageId": null, "replyToAll": false, "to": [], "cc": [], "subject": null,
                  "bodyText": "Body", "budgetMs": 20000 }
            ]),
            &json!({}),
        );
        assert_eq!(runs[0]["result"], json!({ "saved": true }));
        assert_eq!(
            runs[0]["log"],
            json!(["make visible=false subject=Hello", "push message", "set sender", "to a@x.com", "cc c@x.com", "save"])
        );
        // Empty To (Ben fills it in) and no subject are fine.
        assert_eq!(runs[1]["log"], json!(["make visible=false subject=-", "push message", "set sender", "save"]));
    }

    #[test]
    fn mail_quitting_before_the_save_reports_mail_not_running_and_saves_nothing() {
        // Review Focus 3 at the script layer: Mail gone after the reply, before
        // the save → nothing saved, code mail-not-running. An Apple event
        // error -609 (Mail quit mid-event) maps the same way.
        let p = json!({ "mode": "reply", "account": "A", "sender": "", "mailboxes": ["Inbox"],
                        "messageId": "abc@x.com", "replyToAll": false, "to": [], "cc": [], "subject": null,
                        "bodyText": "B", "budgetMs": 20000 });
        let runs = run_draft_in_node(&json!({ "Inbox": [["abc@x.com", 7]] }), &json!([p]), &json!({ "quitAfter": "reply" }));
        assert_eq!(runs[0]["result"], json!({ "saved": false, "code": "mail-not-running" }));
        assert!(!runs[0]["log"].as_array().unwrap().contains(&json!("save")));

        let runs = run_draft_in_node(&json!({ "Inbox": [["abc@x.com", 7]] }), &json!([p]), &json!({ "saveThrows": -609 }));
        assert_eq!(runs[0]["result"], json!({ "saved": false, "code": "mail-not-running" }));
        let runs = run_draft_in_node(&json!({ "Inbox": [["abc@x.com", 7]] }), &json!([p]), &json!({ "saveThrows": -1712 }));
        assert_eq!(runs[0]["result"], json!({ "saved": false, "code": "error:-1712" }));
        let runs = run_draft_in_node(&json!({ "Inbox": [["abc@x.com", 7]] }), &json!([p]), &json!({ "saveThrows": -1743 }));
        assert_eq!(runs[0]["result"], json!({ "saved": false, "code": "permission-denied" }));
    }

    #[test]
    fn a_long_search_stops_at_the_budget() {
        let p = json!({ "mode": "reply", "account": "A", "sender": "", "mailboxes": ["Inbox", "Archive"],
                        "messageId": "abc@x.com", "replyToAll": false, "to": [], "cc": [], "subject": null,
                        "bodyText": "B", "budgetMs": 0 });
        let runs = run_draft_in_node(&json!({ "Inbox": [], "Archive": [["abc@x.com", 1]] }), &json!([p]), &json!({ "slowFetch": true }));
        assert_eq!(runs[0]["result"], json!({ "saved": false, "code": "timeout" }));
        assert_eq!(runs[0]["log"], json!([]));
    }

    /// Run the real draft script in node against a fake `Mail`. Plain JS: no
    /// osascript, no Apple events, nothing near the real Mail.
    /// `boxes`: mailbox → [[messageId, mailId], …]. `opts`: quitAfter
    /// ("reply"), saveThrows (an Apple event error number), slowFetch.
    fn run_draft_in_node(boxes: &serde_json::Value, calls: &serde_json::Value, opts: &serde_json::Value) -> serde_json::Value {
        let harness = format!(
            r#"
var BOXES = {boxes}, OPTS = {opts}, LOG = [], RUNNING = true, LAST = null;
function delay() {{}}
function outgoing(init) {{
  var st = {{ content: init.content || "" }}, o = {{}};
  ["content", "subject", "sender"].forEach(function (k) {{
    Object.defineProperty(o, k, {{
      get: function () {{ return function () {{ return st[k]; }}; }},
      set: function (v) {{ LOG.push("set " + k); st[k] = v; }}
    }});
  }});
  o.toRecipients = {{ push: function (r) {{ LOG.push("to " + r.address); }} }};
  o.ccRecipients = {{ push: function (r) {{ LOG.push("cc " + r.address); }} }};
  o.save = function () {{
    if (OPTS.saveThrows) {{ var e = new Error("no"); e.errorNumber = OPTS.saveThrows; throw e; }}
    LOG.push("save");
  }};
  o.state = st;
  LAST = o;
  return o;
}}
function mailbox(name) {{
  var list = BOXES[name];
  if (!list) {{ var e = new Error("no such mailbox"); e.errorNumber = -1728; throw e; }}
  return {{ messages: {{
    id: function () {{ return list.map(function (m) {{ return m[1]; }}); }},
    messageId: function () {{
      if (OPTS.slowFetch) {{ var t = Date.now(); while (Date.now() - t < 5) {{}} }}
      return list.map(function (m) {{ return m[0]; }});
    }},
    byId: function (id) {{ return {{ reply: function (o) {{
      LOG.push("reply " + id + " all=" + o.replyToAll + (o.openingWindow === false ? "" : " WINDOW"));
      if (OPTS.quitAfter === "reply") RUNNING = false;
      return outgoing({{ content: "> quoted " + id }});
    }} }}; }}
  }} }};
}}
var Mail = {{
  running: function () {{ return RUNNING; }},
  accounts: {{ byId: function () {{ return {{ mailboxes: {{ byName: function (n) {{
    var mb; try {{ mb = mailbox(n); }} catch (e) {{ return {{ messages: {{ id: function () {{ throw e; }}, messageId: function () {{ throw e; }} }} }}; }}
    return mb;
  }} }} }}; }} }},
  OutgoingMessage: function (p) {{
    LOG.push("make visible=" + p.visible + " subject=" + (p.subject === undefined ? "-" : p.subject));
    return outgoing(p);
  }},
  outgoingMessages: {{ push: function () {{ LOG.push("push message"); }} }},
  ToRecipient: function (p) {{ return p; }},
  CcRecipient: function (p) {{ return p; }}
}};
{FIND_FN}
{DRAFT_BODY}
console.log(JSON.stringify({calls}.map(function (P) {{
  LOG = []; RUNNING = true; LAST = null;
  var result = main(P);
  return {{ result: result, log: LOG, content: LAST ? LAST.state.content : null }};
}})));
"#
        );
        let out = std::process::Command::new("node")
            .args(["-e", &harness])
            .output()
            .expect("node is needed for the draft-script test");
        assert!(out.status.success(), "node failed: {}", String::from_utf8_lossy(&out.stderr));
        serde_json::from_slice(&out.stdout).unwrap()
    }
}
