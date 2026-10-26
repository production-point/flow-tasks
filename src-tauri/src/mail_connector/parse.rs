//! Raw Mail message → FLOW `POST /messages` item.
//!
//! Parsing lives in Rust (not the webview) so the loop runs headless while the
//! panel is hidden. Every string and list is clamped to FLOW's server limits:
//! a single over-limit field 400s the whole batch.

use chrono::{DateTime, SecondsFormat, Utc};
use serde::Serialize;

use super::RawMailMessage;

// FLOW server limits (spec §6.2). Server-side "chars" are UTF-16 units;
// clamping by UTF-8 bytes at the same numbers is always within them.
pub const MAX_MESSAGE_ID: usize = 998;
pub const MAX_IN_REPLY_TO: usize = 10_000;
pub const MAX_REFERENCES: usize = 1_000;
pub const MAX_REFERENCE: usize = 10_000;
pub const MAX_ADDRESS: usize = 998;
pub const MAX_RECIPIENTS: usize = 1_000;
pub const MAX_SUBJECT: usize = 10_000;
/// 100 KB.
pub const MAX_BODY_BYTES: usize = 102_400;
pub const MAX_ATTACHMENTS: usize = 500;
pub const MAX_ATTACHMENT_NAME: usize = 1_000;
pub const MAX_LIST_UNSUBSCRIBE: usize = 10_000;
pub const MAX_AUTO_SUBMITTED: usize = 1_000;
pub const MAX_PRECEDENCE: usize = 1_000;
pub const MAX_CONTENT_TYPE: usize = 10_000;
pub const MAX_MAILBOX: usize = 1_000;

/// The headers FLOW cares about, from `allHeaders()`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct ParsedHeaders {
    pub message_id: Option<String>,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub list_unsubscribe: Option<String>,
    pub auto_submitted: Option<String>,
    pub precedence: Option<String>,
    pub content_type: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ConnectorAddress {
    pub address: String,
    pub name: Option<String>,
}

/// Header subset sent to FLOW; absent keys are omitted.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorHeaders {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub list_unsubscribe: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub auto_submitted: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub precedence: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub content_type: Option<String>,
}

/// One item of FLOW's `POST /messages` `messages` array.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorMessage {
    pub internet_message_id: String,
    pub in_reply_to: Option<String>,
    pub references: Vec<String>,
    pub from: Option<ConnectorAddress>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub subject: Option<String>,
    pub received_at: String,
    pub sent_at: Option<String>,
    pub body_text: String,
    pub attachment_names: Vec<String>,
    pub headers: ConnectorHeaders,
}

/// Parse a raw header block: case-insensitive names, folded continuation lines
/// unfolded, first occurrence wins, stops at the first blank line.
pub fn parse_headers(all_headers: &str) -> ParsedHeaders {
    // (lower-cased name, unfolded value), in order.
    let mut fields: Vec<(String, String)> = Vec::new();
    for line in all_headers.split('\n') {
        let line = line.strip_suffix('\r').unwrap_or(line);
        if line.trim().is_empty() {
            if fields.is_empty() {
                continue; // tolerate leading blank lines
            }
            break; // end of the header block
        }
        if line.starts_with(' ') || line.starts_with('\t') {
            // Folded continuation of the previous field.
            if let Some((_, value)) = fields.last_mut() {
                let more = line.trim();
                if !more.is_empty() {
                    if !value.is_empty() {
                        value.push(' ');
                    }
                    value.push_str(more);
                }
            }
            continue;
        }
        if let Some((name, value)) = line.split_once(':') {
            fields.push((name.trim().to_ascii_lowercase(), value.trim().to_string()));
        }
    }

    let first = |name: &str| {
        fields
            .iter()
            .find(|(n, v)| n == name && !v.is_empty())
            .map(|(_, v)| v.as_str())
    };
    let clamped = |name: &str, max: usize| first(name).map(|v| clamp_str(v, max));

    ParsedHeaders {
        message_id: first("message-id").map(first_msg_id).map(|v| clamp_str(&v, MAX_MESSAGE_ID)),
        in_reply_to: first("in-reply-to").map(first_msg_id).map(|v| clamp_str(&v, MAX_IN_REPLY_TO)),
        references: first("references")
            .map(msg_ids)
            .unwrap_or_default()
            .into_iter()
            .take(MAX_REFERENCES)
            .map(|r| clamp_str(&r, MAX_REFERENCE))
            .collect(),
        list_unsubscribe: clamped("list-unsubscribe", MAX_LIST_UNSUBSCRIBE),
        auto_submitted: clamped("auto-submitted", MAX_AUTO_SUBMITTED),
        precedence: clamped("precedence", MAX_PRECEDENCE),
        content_type: clamped("content-type", MAX_CONTENT_TYPE),
    }
}

/// Every `<…>` id in a header value; if there are none, whitespace-separated
/// tokens (some clients omit the brackets).
fn msg_ids(value: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut rest = value;
    while let Some(start) = rest.find('<') {
        match rest[start..].find('>') {
            Some(len) => {
                ids.push(rest[start..start + len + 1].to_string());
                rest = &rest[start + len + 1..];
            }
            None => break,
        }
    }
    if ids.is_empty() {
        ids = value.split_whitespace().map(String::from).collect();
    }
    ids
}

/// The first id of a single-id header (`<a@b> (comment)` → `<a@b>`).
fn first_msg_id(value: &str) -> String {
    msg_ids(value).into_iter().next().unwrap_or_else(|| value.trim().to_string())
}

/// Parse `Name <addr>`, `"Last, First" <addr>`, `<addr>` or a bare `addr`.
/// `None` for an empty sender.
pub fn parse_address(s: &str) -> Option<ConnectorAddress> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    let (address, name) = match s.rfind('<') {
        Some(lt) if s.ends_with('>') => (s[lt + 1..s.len() - 1].trim(), unquote(s[..lt].trim())),
        _ => (s, String::new()),
    };
    // Clean first, then check: an address of only NULs is no address.
    let address = clamp_str(address, MAX_ADDRESS);
    let name = clamp_str(&name, MAX_ADDRESS);
    if address.trim().is_empty() {
        return None;
    }
    Some(ConnectorAddress { address, name: (!name.trim().is_empty()).then_some(name) })
}

/// `"Example, Ann"` → `Example, Ann`, undoing `\"` / `\\` escapes.
fn unquote(s: &str) -> String {
    let Some(inner) = s.strip_prefix('"').and_then(|t| t.strip_suffix('"')) else {
        return s.to_string();
    };
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            if let Some(next) = chars.next() {
                out.push(next);
            }
        } else {
            out.push(c);
        }
    }
    out.trim().to_string()
}

/// Build FLOW's message item, or `None` when the message has no usable
/// Message-ID (Mail's `messageId()`, else the header; empty or bare `<>` is
/// unusable). Such messages are skipped and counted, never sent.
pub fn to_connector_message(raw: &RawMailMessage) -> Option<ConnectorMessage> {
    let headers = parse_headers(&raw.headers);
    let usable = |id: &str| !id.trim().trim_start_matches('<').trim_end_matches('>').trim().is_empty();
    let internet_message_id = raw
        .message_id
        .as_deref()
        .map(|id| clamp_str(id.trim(), MAX_MESSAGE_ID))
        .filter(|id| usable(id))
        .or_else(|| headers.message_id.clone().filter(|id| usable(id)))?;

    let recipients = |list: &[String]| -> Vec<String> {
        list.iter()
            .map(|a| clamp_str(a.trim(), MAX_ADDRESS))
            .filter(|a| !a.trim().is_empty())
            .take(MAX_RECIPIENTS)
            .collect()
    };

    Some(ConnectorMessage {
        internet_message_id,
        in_reply_to: headers.in_reply_to,
        references: headers.references,
        from: raw.sender.as_deref().and_then(parse_address),
        to: recipients(&raw.to),
        cc: recipients(&raw.cc),
        subject: raw.subject.as_deref().map(|s| clamp_str(s, MAX_SUBJECT)),
        received_at: ms_to_iso(raw.date_received),
        sent_at: raw.date_sent.map(ms_to_iso),
        body_text: clamp_str(&raw.content, MAX_BODY_BYTES),
        attachment_names: raw
            .attachments
            .iter()
            .take(MAX_ATTACHMENTS)
            .map(|n| clamp_str(n, MAX_ATTACHMENT_NAME))
            .collect(),
        headers: ConnectorHeaders {
            list_unsubscribe: headers.list_unsubscribe,
            auto_submitted: headers.auto_submitted,
            precedence: headers.precedence,
            content_type: headers.content_type,
        },
    })
}

/// Epoch ms → `2026-10-26T10:00:00.000Z`. Out-of-range values (never produced
/// by Mail) fall back to the epoch rather than panicking.
pub fn ms_to_iso(ms: i64) -> String {
    DateTime::<Utc>::from_timestamp_millis(ms)
        .unwrap_or_default()
        .to_rfc3339_opts(SecondsFormat::Millis, true)
}

/// ISO-8601 / RFC 3339 → epoch ms (FLOW's `resumeFrom` / `highWater`).
pub fn iso_to_ms(s: &str) -> Option<i64> {
    DateTime::parse_from_rfc3339(s.trim()).ok().map(|d| d.timestamp_millis())
}

/// Make a string safe for FLOW: drop NUL (Postgres `text` rejects U+0000, so
/// one would 400 the whole batch), then truncate to at most `max_bytes` UTF-8
/// bytes on a char boundary. Every string field we send goes through here.
pub fn clamp_str(s: &str, max_bytes: usize) -> String {
    let cleaned;
    let s = if s.contains('\0') {
        cleaned = s.replace('\0', "");
        cleaned.as_str()
    } else {
        s
    };
    if s.len() <= max_bytes {
        return s.to_string();
    }
    let mut end = max_bytes;
    while !s.is_char_boundary(end) {
        end -= 1;
    }
    s[..end].to_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn raw(message_id: Option<&str>, headers: &str) -> RawMailMessage {
        RawMailMessage {
            mail_id: Some(1),
            message_id: message_id.map(String::from),
            date_received: 1_761_472_800_000, // 2025-10-26T10:00:00.000Z
            date_sent: Some(1_761_472_740_000),
            sender: Some("Ann Example <ann@example.com>".into()),
            subject: Some("Hello".into()),
            to: vec!["ben@example.com".into()],
            cc: vec![],
            attachments: vec!["a.pdf".into()],
            headers: headers.into(),
            content: "Body".into(),
        }
    }

    #[test]
    fn folded_references_header_parses_all_ids() {
        let h = "From: Ann <ann@example.com>\r\n\
                 References: <one@example.com>\r\n\
                 \t<two@example.com>\r\n   <three@example.com> <four@example.com>\r\n\
                 In-Reply-To:\r\n <four@example.com>\r\n\
                 message-id: <self@example.com>\r\n\
                 LIST-UNSUBSCRIBE: <mailto:u@example.com>,\r\n <https://example.com/u>\r\n\
                 Auto-Submitted: auto-generated\r\n\
                 Precedence: bulk\r\n\
                 Content-Type: multipart/alternative;\r\n\tboundary=\"b1\"\r\n\
                 \r\n\
                 References: <body@example.com>\r\n";
        let p = parse_headers(h);
        assert_eq!(
            p.references,
            vec!["<one@example.com>", "<two@example.com>", "<three@example.com>", "<four@example.com>"]
        );
        assert_eq!(p.in_reply_to.as_deref(), Some("<four@example.com>"));
        assert_eq!(p.message_id.as_deref(), Some("<self@example.com>"));
        assert_eq!(
            p.list_unsubscribe.as_deref(),
            Some("<mailto:u@example.com>, <https://example.com/u>")
        );
        assert_eq!(p.auto_submitted.as_deref(), Some("auto-generated"));
        assert_eq!(p.precedence.as_deref(), Some("bulk"));
        assert_eq!(p.content_type.as_deref(), Some("multipart/alternative; boundary=\"b1\""));
    }

    #[test]
    fn unbracketed_references_split_on_whitespace() {
        let p = parse_headers("References: a@x b@x\n");
        assert_eq!(p.references, vec!["a@x", "b@x"]);
        assert_eq!(parse_headers("").references, Vec::<String>::new());
    }

    #[test]
    fn messages_without_message_id_are_skipped() {
        // RF 4: no messageId() and no header → skipped.
        assert!(to_connector_message(&raw(None, "Subject: x\n")).is_none());
        assert!(to_connector_message(&raw(Some(""), "")).is_none());
        assert!(to_connector_message(&raw(Some("  "), "Message-ID: <>\n")).is_none());
        assert!(to_connector_message(&raw(Some("<>"), "")).is_none());

        // Mail's id wins, sent as Mail gives it (no brackets added).
        let m = to_connector_message(&raw(Some("abc@example.com"), "Message-ID: <hdr@example.com>\n")).unwrap();
        assert_eq!(m.internet_message_id, "abc@example.com");

        // Falls back to the header.
        let m = to_connector_message(&raw(None, "Message-Id: <hdr@example.com>\n")).unwrap();
        assert_eq!(m.internet_message_id, "<hdr@example.com>");
    }

    #[test]
    fn sender_with_display_name_parses() {
        assert_eq!(
            parse_address("Ann Example <ann@example.com>"),
            Some(ConnectorAddress { address: "ann@example.com".into(), name: Some("Ann Example".into()) })
        );
        assert_eq!(
            parse_address("\"Example, Ann\" <ann@example.com>"),
            Some(ConnectorAddress { address: "ann@example.com".into(), name: Some("Example, Ann".into()) })
        );
        assert_eq!(
            parse_address("\"Ann \\\"AJ\\\" Example\" <ann@example.com>"),
            Some(ConnectorAddress { address: "ann@example.com".into(), name: Some("Ann \"AJ\" Example".into()) })
        );
        assert_eq!(
            parse_address("<ann@example.com>"),
            Some(ConnectorAddress { address: "ann@example.com".into(), name: None })
        );
        assert_eq!(
            parse_address(" ann@example.com "),
            Some(ConnectorAddress { address: "ann@example.com".into(), name: None })
        );
        assert_eq!(parse_address(""), None);
        assert_eq!(parse_address("   "), None);

        let m = to_connector_message(&raw(Some("id@x"), "")).unwrap();
        assert_eq!(m.from, Some(ConnectorAddress { address: "ann@example.com".into(), name: Some("Ann Example".into()) }));
    }

    #[test]
    fn body_capped_at_100_kb_on_a_char_boundary() {
        // 3-byte chars: 102,400 isn't a multiple of 3, so a naive byte cut
        // would split one.
        let mut r = raw(Some("id@x"), "");
        r.content = "€".repeat(40_000); // 120,000 bytes
        let m = to_connector_message(&r).unwrap();
        assert!(m.body_text.len() <= MAX_BODY_BYTES);
        assert_eq!(m.body_text.len(), 102_399);
        assert!(m.body_text.chars().all(|c| c == '€'));
        // ≤ 102,400 UTF-16 units too.
        assert!(m.body_text.encode_utf16().count() <= MAX_BODY_BYTES);

        // Short bodies are untouched.
        r.content = "short".into();
        assert_eq!(to_connector_message(&r).unwrap().body_text, "short");

        assert_eq!(clamp_str("aé", 2), "a");
        assert_eq!(clamp_str("aé", 3), "aé");
    }

    #[test]
    fn connector_message_matches_the_wire_shape_and_limits() {
        let mut r = raw(Some("id@x"), "In-Reply-To: <p@x>\nReferences: <p@x>\nPrecedence: bulk\n");
        r.to = (0..1_200).map(|i| format!("u{i}@x")).collect();
        r.cc = vec!["c".repeat(2_000)];
        r.attachments = (0..600).map(|i| format!("{i}.pdf")).collect();
        r.subject = Some("s".repeat(20_000));
        let m = to_connector_message(&r).unwrap();
        assert_eq!(m.to.len(), MAX_RECIPIENTS);
        assert_eq!(m.cc[0].len(), MAX_ADDRESS);
        assert_eq!(m.attachment_names.len(), MAX_ATTACHMENTS);
        assert_eq!(m.subject.as_ref().unwrap().len(), MAX_SUBJECT);

        let v = serde_json::to_value(&m).unwrap();
        assert_eq!(v["internetMessageId"], "id@x");
        assert_eq!(v["inReplyTo"], "<p@x>");
        assert_eq!(v["references"][0], "<p@x>");
        assert_eq!(v["from"]["address"], "ann@example.com");
        assert_eq!(v["receivedAt"], "2025-10-26T10:00:00.000Z");
        assert_eq!(v["sentAt"], "2025-10-26T09:59:00.000Z");
        assert_eq!(v["bodyText"], "Body");
        assert_eq!(v["attachmentNames"][0], "0.pdf");
        assert_eq!(v["headers"], serde_json::json!({ "precedence": "bulk" }));

        // Nulls stay explicit; absent headers are omitted.
        let mut r = raw(Some("id@x"), "");
        r.sender = None;
        r.subject = None;
        r.date_sent = None;
        let v = serde_json::to_value(to_connector_message(&r).unwrap()).unwrap();
        assert!(v["from"].is_null() && v.get("from").is_some());
        assert!(v["subject"].is_null() && v.get("subject").is_some());
        assert!(v["sentAt"].is_null() && v.get("sentAt").is_some());
        assert!(v["inReplyTo"].is_null() && v.get("inReplyTo").is_some());
        assert_eq!(v["headers"], serde_json::json!({}));
    }

    #[test]
    fn nul_is_stripped_from_every_string_field() {
        // Task 2 review follow-up: Postgres text rejects U+0000, so one NUL
        // would 400 the whole batch.
        let r = RawMailMessage {
            mail_id: None,
            message_id: Some("id\u{0}@x".into()),
            date_received: 1_761_472_800_000,
            date_sent: None,
            sender: Some("A\u{0}nn <ann\u{0}@example.com>".into()),
            subject: Some("Sub\u{0}ject".into()),
            to: vec!["t\u{0}@x".into(), "\u{0}".into()],
            cc: vec!["c\u{0}@x".into()],
            attachments: vec!["a\u{0}.pdf".into()],
            headers: "In-Reply-To: <p\u{0}@x>\nReferences: <r\u{0}@x>\nList-Unsubscribe: <u\u{0}>\n\
                      Auto-Submitted: au\u{0}to\nPrecedence: bu\u{0}lk\nContent-Type: te\u{0}xt/plain\n"
                .into(),
            content: "Bo\u{0}dy".into(),
        };
        let m = to_connector_message(&r).unwrap();
        let json = serde_json::to_string(&m).unwrap();
        assert!(!json.contains("\\u0000"), "{json}");
        assert_eq!(m.internet_message_id, "id@x");
        assert_eq!(m.from, Some(ConnectorAddress { address: "ann@example.com".into(), name: Some("Ann".into()) }));
        assert_eq!(m.to, vec!["t@x"], "a NUL-only recipient is dropped");
        assert_eq!(m.body_text, "Body");
        assert_eq!(m.headers.precedence.as_deref(), Some("bulk"));

        // An id that is only NULs is unusable → skipped.
        let mut r2 = r.clone();
        r2.message_id = Some("\u{0}\u{0}".into());
        r2.headers = String::new();
        assert!(to_connector_message(&r2).is_none());
        assert_eq!(clamp_str("a\u{0}b", 10), "ab");
    }

    #[test]
    fn iso_round_trip() {
        assert_eq!(ms_to_iso(0), "1970-01-01T00:00:00.000Z");
        assert_eq!(ms_to_iso(1_761_472_800_123), "2025-10-26T10:00:00.123Z");
        assert_eq!(iso_to_ms("2025-10-26T10:00:00.123Z"), Some(1_761_472_800_123));
        assert_eq!(iso_to_ms("2025-10-26T11:00:00+01:00"), Some(1_761_472_800_000));
        assert_eq!(iso_to_ms("yesterday"), None);
        assert_eq!(ms_to_iso(i64::MAX), "1970-01-01T00:00:00.000Z");
    }
}
