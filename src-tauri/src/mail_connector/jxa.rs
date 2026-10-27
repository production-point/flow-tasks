//! JXA script text and the osascript runner.
//!
//! Every script is prelude + body + epilogue. The body defines `main(P)`; the
//! epilogue checks `Mail.running()` first — a local check that sends no Apple
//! event and never launches Mail — and wraps the result as `{"ok": …}` or
//! `{"notRunning": true}`. Parameters are injected as one JSON literal (`P`),
//! never string-interpolated, so an odd account id or mailbox name can't
//! break out of the script.
//!
//! The scripts here are read-only: no move / delete / flag / send / draft /
//! make / save, anywhere (`drafts::tests` pins this). The one write path, the
//! draft script, lives in `drafts.rs` with its own verb allowlist. Semantics were verified against Apple Mail on Ben's Mac
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

/// P: account, mailbox, since (epoch ms), afterId (Mail id or null), limit,
/// budgetMs. Due = received after `since`, or at exactly `since` with an id
/// above `afterId`: a compound cursor, so a chunk cut inside a group of
/// same-second messages resumes inside the group. Sorted by (date, id).
/// Returns `{ messages (oldest first, each with its Mail `mailId`), more,
/// notRunning?, errorCode? }`.
///
/// Only a genuinely missing property reads as null: Mail returns missing
/// values as null itself, and -1728 ("can't get") means the property or the
/// message isn't there. Any other error (an Apple event timeout, Mail
/// quitting) ends the chunk *before* that message, with `errorCode` set, so
/// it's read again next tick rather than posted half-empty: FLOW dedupes by
/// Message-ID and would never take the full copy.
pub const READ_MAILBOX_BODY: &str = r#"function main(P) {
  var mb = Mail.accounts.byId(P.account).mailboxes.byName(P.mailbox);
  var ids = mb.messages.id(), dates = mb.messages.dateReceived();
  if (ids.length !== dates.length) { ids = mb.messages.id(); dates = mb.messages.dateReceived(); }
  if (ids.length !== dates.length) throw new Error("mailbox changed while reading");
  var due = [];
  for (var i = 0; i < ids.length; i++) {
    var t = dates[i] ? dates[i].getTime() : 0;
    if (t > P.since || (t === P.since && P.afterId !== null && ids[i] > P.afterId)) due.push([t, ids[i]]);
  }
  due.sort(function (a, b) { return a[0] - b[0] || a[1] - b[1]; });
  var start = Date.now(), out = [];
  function get(f) { try { var v = f(); return v === undefined ? null : v; } catch (e) { if (e.errorNumber === -1728) return null; throw e; } }
  for (var j = 0; j < due.length && out.length < P.limit; j++) {
    if (!Mail.running()) return { messages: out, more: true, notRunning: true };
    if (out.length > 0 && Date.now() - start > P.budgetMs) break;
    var m = mb.messages.byId(due[j][1]);
    try {
      var sent = get(function () { return m.dateSent(); });
      out.push({
        mailId: due[j][1],
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
    } catch (e) {
      return { messages: out, more: true, errorCode: typeof e.errorNumber === "number" ? e.errorNumber : 0 };
    }
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
    // "… Not authorized to send Apple events to Mail. (-1743)". Only that
    // trailing code counts: the text before it can quote a subject.
    classify_error_code(trailing_error_code(stderr))
}

/// Map an Apple event error code (from osascript, or caught inside the read
/// script) to an error class.
pub fn classify_error_code(code: Option<i64>) -> ScriptError {
    match code {
        Some(-1743) => ScriptError::PermissionDenied,
        // -600 "Application isn't running"; -609 "Connection is invalid",
        // which is what Mail gives when it quits while a script is talking
        // to it.
        Some(-600 | -609) => ScriptError::NotRunning,
        Some(c) => ScriptError::Other(format!("osascript error {c}")),
        None => ScriptError::Other("osascript failed".into()),
    }
}

/// The `(-NNNN)` that ends the text, as a number.
fn trailing_error_code(s: &str) -> Option<i64> {
    let s = s.trim_end().strip_suffix(')')?;
    let digits = &s[s.rfind("(-")? + 2..];
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse::<i64>().ok().map(|n| -n)
}

/// Wall-clock limit for one osascript run. Each read asks the script to stop
/// after 20 s, but that budget is only checked *between* Apple events; a hung
/// Mail can hold a single event for ~2 min. This only fires in that case.
#[cfg(target_os = "macos")]
pub const OSASCRIPT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(90);

/// Run `cmd` to completion, killing it after `timeout`
/// (→ `Other("osascript timeout")`). stdout/stderr are drained on their own
/// threads while we wait: a read chunk can be megabytes, and a child blocked
/// on a full pipe would otherwise look hung.
#[cfg(target_os = "macos")]
pub fn run_with_timeout(
    mut cmd: std::process::Command,
    timeout: std::time::Duration,
) -> Result<std::process::Output, ScriptError> {
    use std::io::Read;
    use std::process::Stdio;
    use std::time::{Duration, Instant};

    let mut child = cmd
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| ScriptError::Other(format!("osascript spawn failed: {:?}", e.kind())))?;
    let drain = |pipe: Option<Box<dyn Read + Send>>| {
        std::thread::spawn(move || {
            let mut buf = Vec::new();
            if let Some(mut p) = pipe {
                let _ = p.read_to_end(&mut buf);
            }
            buf
        })
    };
    let out = drain(child.stdout.take().map(|p| Box::new(p) as Box<dyn Read + Send>));
    let err = drain(child.stderr.take().map(|p| Box::new(p) as Box<dyn Read + Send>));

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() < deadline => std::thread::sleep(Duration::from_millis(20)),
            Ok(None) => {
                let _ = child.kill();
                let _ = child.wait();
                // The pipes close with the process, so the drains finish.
                let _ = (out.join(), err.join());
                return Err(ScriptError::Other("osascript timeout".into()));
            }
            Err(e) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(ScriptError::Other(format!("osascript wait failed: {:?}", e.kind())));
            }
        }
    };
    Ok(std::process::Output {
        status,
        stdout: out.join().unwrap_or_default(),
        stderr: err.join().unwrap_or_default(),
    })
}

/// The real runner: `osascript -l JavaScript -e <script>`. The only code in
/// the app that spawns osascript.
pub struct OsascriptRunner;

impl super::ScriptRunner for OsascriptRunner {
    #[cfg(target_os = "macos")]
    fn run_jxa(&self, script: &str) -> Result<String, ScriptError> {
        let mut cmd = std::process::Command::new("/usr/bin/osascript");
        cmd.args(["-l", "JavaScript", "-e", script]);
        let out = run_with_timeout(cmd, OSASCRIPT_TIMEOUT)?;
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
    fn maps_609_to_not_running() {
        // Task 2 review follow-up: Mail quitting mid-script gives -609.
        let e = classify_osascript_failure("execution error: Error: Connection is invalid. (-609)\n");
        assert_eq!(e, ScriptError::NotRunning);
        // -6090 is not -609.
        assert_eq!(classify_osascript_failure("boom (-6090)"), ScriptError::Other("osascript error -6090".into()));
    }

    #[test]
    fn only_the_trailing_error_code_counts() {
        // Review: a quoted subject must not decide the class.
        let e = classify_osascript_failure(
            "execution error: Error: Can't get message \"INV-1743 (-1743)\". (-1712)\n",
        );
        assert_eq!(e, ScriptError::Other("osascript error -1712".into()));
        assert_eq!(classify_osascript_failure("execution error: INV-1743 went wrong"), ScriptError::Other("osascript failed".into()));
        assert_eq!(classify_osascript_failure("boom (-1743) and then more text"), ScriptError::Other("osascript failed".into()));
        assert_eq!(classify_osascript_failure("Not authorized. (-1743)  \n\n"), ScriptError::PermissionDenied);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn a_hung_osascript_is_killed_after_the_timeout() {
        // Uses /bin/sleep as a stand-in for a hung osascript: nothing talks to Mail.
        let start = std::time::Instant::now();
        let mut cmd = std::process::Command::new("/bin/sleep");
        cmd.arg("5");
        let e = run_with_timeout(cmd, std::time::Duration::from_millis(200)).unwrap_err();
        assert_eq!(e, ScriptError::Other("osascript timeout".into()));
        assert!(start.elapsed() < std::time::Duration::from_secs(3));

        // A process that finishes in time returns its output, even a large one
        // (the pipes are drained while waiting, so a full pipe can't hang it).
        let mut cmd = std::process::Command::new("/bin/sh");
        cmd.args(["-c", "head -c 300000 /dev/zero | tr '\\0' x"]);
        let out = run_with_timeout(cmd, std::time::Duration::from_secs(10)).unwrap();
        assert!(out.status.success());
        assert_eq!(out.stdout.len(), 300_000);
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

    /// Review I1: run the real read body in node against a fake `Mail`
    /// object. Plain JS: no osascript, no Apple events, nothing near Mail.
    fn run_read_body_in_node(calls: &serde_json::Value) -> serde_json::Value {
        run_read_body_in_node_failing(calls, &json!({}))
    }

    /// `fails`: Mail id → { property: Apple event error number it throws }.
    fn run_read_body_in_node_failing(calls: &serde_json::Value, fails: &serde_json::Value) -> serde_json::Value {
        let harness = format!(
            r#"
var DATES = [3000, 1000, 2000, 2000, 2000, 2000, 4000];
var IDS =   [  70,   10,   24,   21,   23,   22,   80];
var FAILS = {fails};
function msg(id) {{
  function prop(name, v) {{
    return function () {{
      var code = (FAILS[id] || {{}})[name];
      if (code !== undefined) {{ var e = new Error("Mail said no"); e.errorNumber = code; throw e; }}
      return v;
    }};
  }}
  return {{
    messageId: prop("messageId", "m" + id), dateSent: prop("dateSent", null),
    sender: prop("sender", null), subject: prop("subject", null),
    toRecipients: {{ address: prop("to", []) }}, ccRecipients: {{ address: prop("cc", []) }},
    mailAttachments: {{ name: prop("attachments", []) }},
    allHeaders: prop("allHeaders", ""), content: prop("content", "body " + id)
  }};
}}
var MB = {{ messages: {{
  id: function () {{ return IDS; }},
  dateReceived: function () {{ return DATES.map(function (t) {{ return new Date(t); }}); }},
  byId: function (id) {{ return msg(id); }}
}} }};
var Mail = {{ running: function () {{ return true; }},
  accounts: {{ byId: function () {{ return {{ mailboxes: {{ byName: function () {{ return MB; }} }} }}; }} }} }};
{READ_MAILBOX_BODY}
console.log(JSON.stringify({calls}.map(function (P) {{ return main(P); }})));
"#
        );
        let out = std::process::Command::new("node")
            .args(["-e", &harness])
            .output()
            .expect("node is needed for the read-script test (it ships with the app's toolchain)");
        assert!(out.status.success(), "node failed");
        serde_json::from_slice(&out.stdout).unwrap()
    }

    #[test]
    fn read_script_resumes_inside_a_same_second_group() {
        let p = |since: i64, after: serde_json::Value, limit: u32| {
            json!({ "account": "A", "mailbox": "Inbox", "since": since, "afterId": after, "limit": limit, "budgetMs": 20000 })
        };
        let runs = run_read_body_in_node(&json!([
            p(1000, serde_json::Value::Null, 2), // into the 2000 group: 21, 22
            p(2000, json!(22), 2),               // rest of the group: 23, 24
            p(2000, json!(24), 10),              // past the group
            p(2000, serde_json::Value::Null, 10) // no afterId: strictly after 2000
        ]));
        let ids = |i: usize| runs[i]["messages"].as_array().unwrap().iter().map(|m| m["mailId"].as_i64().unwrap()).collect::<Vec<_>>();
        assert_eq!(ids(0), vec![21, 22]);
        assert_eq!(runs[0]["more"], true);
        assert_eq!(runs[0]["messages"][0]["dateReceived"], 2000);
        assert_eq!(ids(1), vec![23, 24]);
        assert_eq!(runs[1]["more"], true);
        assert_eq!(ids(2), vec![70, 80]);
        assert_eq!(runs[2]["more"], false);
        assert_eq!(ids(3), vec![70, 80]);
    }

    #[test]
    fn a_failed_property_read_stops_the_chunk_at_that_message() {
        // PR review item 3: a timeout (-1712) or Mail quitting (-609) mid-read
        // must not turn into a null Message-ID (skipped for good) or an empty
        // body (FLOW dedupes, so it never gets the real one). Messages before
        // the failure are kept; the failed one isn't returned.
        let p = json!({ "account": "A", "mailbox": "Inbox", "since": 0, "afterId": null, "limit": 10, "budgetMs": 20000 });
        for (fails, code) in [
            (json!({ "21": { "content": -1712 } }), -1712),
            (json!({ "21": { "messageId": -609 } }), -609),
            (json!({ "21": { "allHeaders": -1700 } }), -1700),
        ] {
            let runs = run_read_body_in_node_failing(&json!([p]), &fails);
            let run = &runs[0];
            assert_eq!(run["messages"].as_array().unwrap().iter().map(|m| m["mailId"].as_i64().unwrap()).collect::<Vec<_>>(), vec![10]);
            assert_eq!(run["more"], true);
            assert_eq!(run["errorCode"], code);
        }

        // A JS error without an Apple event number still stops the chunk.
        // (The harness throws `errorNumber: null` for a null code.)
        let runs = run_read_body_in_node_failing(&json!([p]), &json!({ "21": { "subject": null } }));
        assert_eq!(runs[0]["messages"].as_array().unwrap().len(), 1);
        assert_eq!(runs[0]["errorCode"], 0);

        // The first due message failing gives an empty chunk that still
        // says so.
        let runs = run_read_body_in_node_failing(&json!([p]), &json!({ "10": { "content": -1712 } }));
        assert!(runs[0]["messages"].as_array().unwrap().is_empty());
        assert_eq!(runs[0]["more"], true);
        assert_eq!(runs[0]["errorCode"], -1712);

        // -1728 ("can't get": the property, or the message, isn't there) is
        // a genuinely missing value: null, and the read goes on.
        let runs = run_read_body_in_node_failing(&json!([p]), &json!({ "21": { "dateSent": -1728, "content": -1728 } }));
        let msgs = runs[0]["messages"].as_array().unwrap();
        assert_eq!(msgs.len(), 7);
        assert_eq!(msgs[1]["mailId"], 21);
        assert_eq!(msgs[1]["dateSent"], serde_json::Value::Null);
        assert_eq!(msgs[1]["content"], "");
        assert!(runs[0].get("errorCode").is_none());
    }
}
