//! JXA script text and the osascript runner.
//!
//! Every script is prelude + body + epilogue. The body defines `main(P)`; the
//! epilogue checks `Mail.running()` first — a local check that sends no Apple
//! event and never launches Mail — and wraps the result as `{"ok": …}` or
//! `{"notRunning": true}`. Parameters are injected as one JSON literal (`P`),
//! never string-interpolated, so an odd account id or mailbox name can't
//! break out of the script.
//!
//! The scripts are read-only: no move / delete / flag / send / draft / make /
//! save, anywhere. Semantics were verified against Apple Mail on Ben's Mac
//! (see design notes): notably we never use `whose(...)` — it's re-evaluated on
//! every property read and froze Mail for 35 s in one Apple event. Instead the
//! read script fetches ids + dates for the whole mailbox (~120 ms each for
//! 4,762 messages), filters in JS, and reads due messages one by one by id
//! under a time budget.

use super::ScriptError;

/// `ping`: proves Mail is running and we're allowed to talk to it.
pub const PING_BODY: &str = r#"function main(P) { return true; }"#;

pub const LIST_ACCOUNTS_BODY: &str = r#"function main(P) {
  var a = Mail.accounts;
  var ids = a.id(), names = a.name(), types = a.accountType(), addrs = a.emailAddresses();
  return ids.map(function (id, i) { return { id: id, name: names[i], kind: String(types[i]), addresses: addrs[i] || [] }; });
}"#;

/// `acct.mailboxes()` is flat (nested ones included); the full path of a nested
/// mailbox is only visible as the `byName("…")` argument in its display string.
pub const LIST_MAILBOXES_BODY: &str = r#"function main(P) {
  var mbs = Mail.accounts.byId(P.account).mailboxes();
  return mbs.map(function (mb) {
    var m = /mailboxes\.byName\(("(?:[^"\\]|\\.)*")\)$/.exec(Automation.getDisplayString(mb));
    return m ? JSON.parse(m[1]) : mb.name();
  });
}"#;

/// P: account, mailbox, since (epoch ms, exclusive), limit, budgetMs.
/// Returns `{ messages (oldest first), more, notRunning? }`.
pub const READ_MAILBOX_BODY: &str = r#"function main(P) {
  var mb = Mail.accounts.byId(P.account).mailboxes.byName(P.mailbox);
  var ids = mb.messages.id(), dates = mb.messages.dateReceived();
  if (ids.length !== dates.length) { ids = mb.messages.id(); dates = mb.messages.dateReceived(); }
  if (ids.length !== dates.length) throw new Error("mailbox changed while reading");
  var due = [];
  for (var i = 0; i < ids.length; i++) {
    var t = dates[i] ? dates[i].getTime() : 0;
    if (t > P.since) due.push([t, ids[i]]);
  }
  due.sort(function (a, b) { return a[0] - b[0] || a[1] - b[1]; });
  var start = Date.now(), out = [];
  function get(f) { try { var v = f(); return v === undefined ? null : v; } catch (e) { return null; } }
  for (var j = 0; j < due.length && out.length < P.limit; j++) {
    if (!Mail.running()) return { messages: out, more: true, notRunning: true };
    if (out.length > 0 && Date.now() - start > P.budgetMs) break;
    var m = mb.messages.byId(due[j][1]);
    var sent = get(function () { return m.dateSent(); });
    out.push({
      messageId: get(function () { return m.messageId(); }),
      dateReceived: due[j][0],
      dateSent: sent ? sent.getTime() : null,
      sender: get(function () { return m.sender(); }),
      subject: get(function () { return m.subject(); }),
      to: get(function () { return m.toRecipients.address(); }) || [],
      cc: get(function () { return m.ccRecipients.address(); }) || [],
      attachments: get(function () { return m.mailAttachments.name(); }) || [],
      headers: get(function () { return m.allHeaders(); }) || "",
      content: get(function () { return m.content(); }) || ""
    });
  }
  return { messages: out, more: out.length < due.length };
}"#;

const EPILOGUE: &str = r#"(function () { if (!Mail.running()) return JSON.stringify({ notRunning: true }); return JSON.stringify({ ok: main(P) }); })();"#;

/// Assemble prelude + body + epilogue with `params` as the `P` literal.
pub fn build_script(body: &str, params: &serde_json::Value) -> String {
    // serde_json output is a valid JS expression, except that U+2028/U+2029
    // are only legal inside JS string literals from ES2019 on — escape them so
    // the literal parses on any JavaScriptCore.
    let p = serde_json::to_string(params)
        .unwrap_or_else(|_| "{}".into())
        .replace('\u{2028}', "\\u2028")
        .replace('\u{2029}', "\\u2029");
    format!("var Mail = Application(\"Mail\");\nvar P = {p};\n{body}\n{EPILOGUE}\n")
}

/// Map a failed osascript run to an error class. Pure, so it's unit-tested.
/// Never puts stderr text in the error — it can quote message content.
pub fn classify_osascript_failure(stderr: &str) -> ScriptError {
    // osascript ends its error line with the Apple event error code, e.g.
    // "… Not authorized to send Apple events to Mail. (-1743)".
    let code = last_error_code(stderr);
    if code == Some(-1743) || stderr.contains("-1743") {
        return ScriptError::PermissionDenied;
    }
    if code == Some(-600) {
        return ScriptError::NotRunning;
    }
    match code {
        Some(c) => ScriptError::Other(format!("osascript error {c}")),
        None => ScriptError::Other("osascript failed".into()),
    }
}

/// The last `(-NNNN)` in the text, as a number.
fn last_error_code(s: &str) -> Option<i32> {
    s.rmatch_indices("(-").find_map(|(i, _)| {
        let rest = &s[i + 1..];
        let end = rest[1..].find(|c: char| !c.is_ascii_digit())? + 1;
        if end > 1 && rest[end..].starts_with(')') {
            rest[..end].parse().ok()
        } else {
            None
        }
    })
}

/// The real runner: `osascript -l JavaScript -e <script>`. The only code in
/// the app that spawns osascript.
pub struct OsascriptRunner;

impl super::ScriptRunner for OsascriptRunner {
    #[cfg(target_os = "macos")]
    fn run_jxa(&self, script: &str) -> Result<String, ScriptError> {
        let out = std::process::Command::new("/usr/bin/osascript")
            .args(["-l", "JavaScript", "-e", script])
            .output()
            .map_err(|e| ScriptError::Other(format!("osascript spawn failed: {:?}", e.kind())))?;
        if out.status.success() {
            Ok(String::from_utf8_lossy(&out.stdout).into_owned())
        } else {
            Err(classify_osascript_failure(&String::from_utf8_lossy(&out.stderr)))
        }
    }

    #[cfg(not(target_os = "macos"))]
    fn run_jxa(&self, _script: &str) -> Result<String, ScriptError> {
        Err(ScriptError::Other("unsupported".into()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn maps_1743_to_permission_denied() {
        // RF 2.
        let e = classify_osascript_failure(
            "execution error: Not authorized to send Apple events to Mail. (-1743)\n",
        );
        assert_eq!(e, ScriptError::PermissionDenied);
    }

    #[test]
    fn maps_600_to_not_running() {
        let e = classify_osascript_failure("execution error: Error: Application isn't running. (-600)\n");
        assert_eq!(e, ScriptError::NotRunning);
    }

    #[test]
    fn other_failures_carry_the_code_but_never_the_text() {
        let e = classify_osascript_failure(
            "execution error: Error: Can't get message \"Quarterly results for Acme\". (-1712)\n",
        );
        assert_eq!(e, ScriptError::Other("osascript error -1712".into()));

        let e = classify_osascript_failure("something about Acme went wrong");
        assert_eq!(e, ScriptError::Other("osascript failed".into()));
        assert!(!e.to_string().contains("Acme"));

        // -6000 is not -600.
        let e = classify_osascript_failure("execution error: boom (-6000)");
        assert_eq!(e, ScriptError::Other("osascript error -6000".into()));
    }

    #[test]
    fn params_are_injected_as_a_json_literal() {
        let s = build_script(
            PING_BODY,
            &json!({ "account": "x\"; Mail.quit(); \"", "mailbox": "a\u{2028}b" }),
        );
        let p = crate::mail_connector::tests::params_of(&s);
        assert_eq!(p["account"], "x\"; Mail.quit(); \"");
        assert_eq!(p["mailbox"], "a\u{2028}b");
        assert!(!s.contains('\u{2028}'));
        assert!(s.starts_with("var Mail = Application(\"Mail\");\n"));
    }

    #[test]
    fn scripts_are_read_only() {
        for body in [PING_BODY, LIST_ACCOUNTS_BODY, LIST_MAILBOXES_BODY, READ_MAILBOX_BODY] {
            for verb in [".delete(", ".move(", "Mail.make(", ".save(", ".send(", "flaggedStatus", "readStatus", "whose("] {
                assert!(!body.contains(verb), "script must not use {verb}");
            }
        }
    }
}
