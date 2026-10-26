//! FLOW's assistant connector API (`{flowUrl}/api/v1/assistant/connector`).
//!
//! The runner talks to FLOW only through the `FlowApi` trait so every rule
//! (409 → disconnected, 401 → stop, 429 → keep marks …) is unit-tested against
//! a fake; `HttpFlowApi` is the thin real implementation.
//!
//! Errors are status classes only. Response bodies are never read into an
//! error: FLOW's 400 bodies can echo the offending field, which may be
//! message content.

use std::collections::BTreeMap;
use std::time::Duration;

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};

use super::parse::ConnectorMessage;

/// Path under the configured FLOW URL.
pub const API_BASE: &str = "/api/v1/assistant/connector";
/// Per-request timeout. A run posts many batches; one hung request mustn't
/// hold the loop for longer than this.
pub const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// One account of `POST /accounts`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountRegistration {
    /// The account's first valid email address.
    pub address: String,
    /// All of its valid addresses (≤50).
    pub addresses: Vec<String>,
    /// The Mail account name (≤200).
    pub display_name: String,
    pub apple_account_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct AccountsRequest<'a> {
    accounts: &'a [AccountRegistration],
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountsResponse {
    pub accounts: Vec<RegisteredAccount>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionStatus {
    Connected,
    Disconnected,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct RegisteredAccount {
    /// Lower-cased by FLOW; match back by lower-cased address.
    pub address: String,
    pub connection_id: String,
    pub status: ConnectionStatus,
    /// Mailbox path (as the helper posted it) → ISO time to resume from.
    /// `null` when FLOW has nothing for this connection.
    #[serde(default)]
    pub resume_from: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessagesRequest<'a> {
    pub connection_id: &'a str,
    pub mailbox: &'a str,
    pub messages: &'a [ConnectorMessage],
}

#[derive(Debug, Clone, PartialEq, Eq, Default, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PostResult {
    #[serde(default)]
    pub accepted: u32,
    #[serde(default)]
    pub duplicates: u32,
    /// FLOW's newest stored `receivedAt` for this mailbox (ISO). The runner
    /// advances its mark to the chunk's newest `dateReceived` instead, which is
    /// always ≥ this and also moves past skipped messages.
    #[serde(default)]
    pub high_water: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum HeartbeatStatus {
    Ok,
    MailNotRunning,
    PermissionDenied,
    Error,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
struct HeartbeatRequest<'a> {
    connection_id: &'a str,
    status: HeartbeatStatus,
    /// An error class (≤500), never content.
    #[serde(skip_serializing_if = "Option::is_none")]
    detail: Option<&'a str>,
}

/// FLOW failures, by what the runner does about them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApiError {
    /// 401 / 403: the connector key is wrong or revoked — stop the whole run.
    KeyRejected,
    /// 409: the connection was disconnected in FLOW (or, on `/accounts`,
    /// registration refused).
    Conflict,
    /// 400: FLOW rejected the body.
    BadRequest,
    /// 413: body too large.
    TooLarge,
    /// 429, 5xx, network failure or timeout: try again next tick.
    Unavailable,
    /// Anything else, as a content-free class (`"http 404"`, …).
    Other(String),
}

/// Map a non-2xx HTTP status to its class. Pure, so it's unit-tested.
pub fn classify_status(status: u16) -> ApiError {
    match status {
        400 => ApiError::BadRequest,
        401 | 403 => ApiError::KeyRejected,
        409 => ApiError::Conflict,
        413 => ApiError::TooLarge,
        429 | 500..=599 => ApiError::Unavailable,
        s => ApiError::Other(format!("http {s}")),
    }
}

pub trait FlowApi {
    fn post_accounts(&self, accounts: &[AccountRegistration]) -> Result<AccountsResponse, ApiError>;
    fn post_messages(
        &self,
        connection_id: &str,
        mailbox: &str,
        messages: &[ConnectorMessage],
    ) -> Result<PostResult, ApiError>;
    fn heartbeat(
        &self,
        connection_id: &str,
        status: HeartbeatStatus,
        detail: Option<&str>,
    ) -> Result<(), ApiError>;
}

/// Serialised size of a `POST /messages` body, for the 8 MB batch cap.
pub fn messages_body_len(connection_id: &str, mailbox: &str, messages: &[ConnectorMessage]) -> usize {
    serde_json::to_vec(&MessagesRequest { connection_id, mailbox, messages })
        .map(|v| v.len())
        .unwrap_or(usize::MAX)
}

/// The real client: blocking reqwest (the Tauri HTTP plugin's), Bearer auth,
/// 30 s timeout, no redirects (a redirect would carry the key somewhere we
/// didn't configure).
pub struct HttpFlowApi {
    client: tauri_plugin_http::reqwest::blocking::Client,
    base: String,
    key: String,
}

impl HttpFlowApi {
    /// `flow_url` is the validated base URL (no trailing `/`).
    pub fn new(flow_url: &str, key: &str) -> Result<Self, ApiError> {
        use tauri_plugin_http::reqwest;
        let client = reqwest::blocking::Client::builder()
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .map_err(|_| ApiError::Other("http client".into()))?;
        Ok(HttpFlowApi { client, base: format!("{flow_url}{API_BASE}"), key: key.to_string() })
    }

    /// POST JSON; returns the response body bytes on 2xx.
    fn post<B: Serialize>(&self, path: &str, body: &B) -> Result<Vec<u8>, ApiError> {
        use tauri_plugin_http::reqwest::header::CONTENT_TYPE;
        let bytes = serde_json::to_vec(body).map_err(|_| ApiError::Other("serialise".into()))?;
        let resp = self
            .client
            .post(format!("{}{path}", self.base))
            .bearer_auth(&self.key)
            .header(CONTENT_TYPE, "application/json")
            .body(bytes)
            .send()
            // Network, DNS, TLS, timeout: all "try again later". The reqwest
            // error text is dropped (it names the URL; classes only).
            .map_err(|_| ApiError::Unavailable)?;
        let status = resp.status().as_u16();
        if !(200..300).contains(&status) {
            return Err(classify_status(status));
        }
        resp.bytes().map(|b| b.to_vec()).map_err(|_| ApiError::Unavailable)
    }

    fn post_parse<B: Serialize, T: DeserializeOwned>(&self, path: &str, body: &B) -> Result<T, ApiError> {
        let bytes = self.post(path, body)?;
        serde_json::from_slice(&bytes).map_err(|_| ApiError::Other("unparseable response".into()))
    }
}

impl FlowApi for HttpFlowApi {
    fn post_accounts(&self, accounts: &[AccountRegistration]) -> Result<AccountsResponse, ApiError> {
        self.post_parse("/accounts", &AccountsRequest { accounts })
    }

    fn post_messages(
        &self,
        connection_id: &str,
        mailbox: &str,
        messages: &[ConnectorMessage],
    ) -> Result<PostResult, ApiError> {
        self.post_parse("/messages", &MessagesRequest { connection_id, mailbox, messages })
    }

    fn heartbeat(
        &self,
        connection_id: &str,
        status: HeartbeatStatus,
        detail: Option<&str>,
    ) -> Result<(), ApiError> {
        self.post("/heartbeat", &HeartbeatRequest { connection_id, status, detail }).map(|_| ())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn http_statuses_map_to_error_classes() {
        assert_eq!(classify_status(400), ApiError::BadRequest);
        assert_eq!(classify_status(401), ApiError::KeyRejected);
        assert_eq!(classify_status(403), ApiError::KeyRejected);
        assert_eq!(classify_status(409), ApiError::Conflict);
        assert_eq!(classify_status(413), ApiError::TooLarge);
        assert_eq!(classify_status(429), ApiError::Unavailable);
        assert_eq!(classify_status(500), ApiError::Unavailable);
        assert_eq!(classify_status(503), ApiError::Unavailable);
        assert_eq!(classify_status(404), ApiError::Other("http 404".into()));
        assert_eq!(classify_status(302), ApiError::Other("http 302".into()));
    }

    #[test]
    fn wire_types_match_the_connector_api() {
        let regs = vec![AccountRegistration {
            address: "ben@sparks.example".into(),
            addresses: vec!["ben@sparks.example".into(), "b@sparks.example".into()],
            display_name: "SPARKS".into(),
            apple_account_id: "ABC".into(),
        }];
        assert_eq!(
            serde_json::to_value(AccountsRequest { accounts: &regs }).unwrap(),
            json!({ "accounts": [{
                "address": "ben@sparks.example",
                "addresses": ["ben@sparks.example", "b@sparks.example"],
                "displayName": "SPARKS",
                "appleAccountId": "ABC"
            }] })
        );

        let resp: AccountsResponse = serde_json::from_value(json!({ "accounts": [
            { "address": "ben@sparks.example", "connectionId": "c1", "status": "connected",
              "resumeFrom": { "Inbox": "2026-10-26T10:00:00.000Z" } },
            { "address": "x@y", "connectionId": "c2", "status": "disconnected", "resumeFrom": null }
        ] }))
        .unwrap();
        assert_eq!(resp.accounts[0].status, ConnectionStatus::Connected);
        assert_eq!(resp.accounts[0].resume_from.as_ref().unwrap()["Inbox"], "2026-10-26T10:00:00.000Z");
        assert_eq!(resp.accounts[1].status, ConnectionStatus::Disconnected);
        assert_eq!(resp.accounts[1].resume_from, None);

        let post: PostResult =
            serde_json::from_value(json!({ "accepted": 3, "duplicates": 1, "highWater": null })).unwrap();
        assert_eq!(post, PostResult { accepted: 3, duplicates: 1, high_water: None });

        assert_eq!(
            serde_json::to_value(HeartbeatRequest { connection_id: "c1", status: HeartbeatStatus::MailNotRunning, detail: None }).unwrap(),
            json!({ "connectionId": "c1", "status": "mail-not-running" })
        );
        assert_eq!(
            serde_json::to_value(HeartbeatRequest { connection_id: "c1", status: HeartbeatStatus::Error, detail: Some("osascript error -1712") }).unwrap(),
            json!({ "connectionId": "c1", "status": "error", "detail": "osascript error -1712" })
        );
        assert_eq!(serde_json::to_value(HeartbeatStatus::PermissionDenied).unwrap(), "permission-denied");
        assert_eq!(serde_json::to_value(HeartbeatStatus::Ok).unwrap(), "ok");

        let body = MessagesRequest { connection_id: "c1", mailbox: "Inbox", messages: &[] };
        assert_eq!(
            serde_json::to_value(&body).unwrap(),
            json!({ "connectionId": "c1", "mailbox": "Inbox", "messages": [] })
        );
        assert_eq!(messages_body_len("c1", "Inbox", &[]), serde_json::to_vec(&body).unwrap().len());
    }

    #[test]
    fn the_http_client_targets_the_connector_base() {
        // No request is sent: construction only.
        let api = HttpFlowApi::new("http://127.0.0.1:9", "k").unwrap();
        assert_eq!(api.base, "http://127.0.0.1:9/api/v1/assistant/connector");
    }
}
