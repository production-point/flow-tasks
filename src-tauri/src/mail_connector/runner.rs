//! The background loop: every 5 minutes, read the ticked Mail accounts and
//! post new mail to FLOW.
//!
//! `run_once` is one tick. It is generic over the osascript boundary
//! (`ScriptRunner`), FLOW (`FlowApi`) and time (`Clock`), and works on an
//! in-memory copy of the config and marks, so every rule below is unit-tested
//! without Mail, the network, the keychain or real sleeps. `tick` wraps it
//! with the real implementations and persists the result.
//!
//! Rules (design notes, "Runner rules"):
//! - ping first; Mail not running / Automation denied → one heartbeat per
//!   known connection, no reads.
//! - Register every selected account each tick; FLOW's `resumeFrom` (keyed by
//!   the mailbox path we posted) wins over local marks.
//! - Read each included mailbox in chunks (≤200, 20 s script budget, 250 ms
//!   between osascript calls), post in batches (≤100 messages, ≤8 MB), and
//!   advance the mark only once every batch of a chunk has posted.
//! - Per account per run: stop after 2,000 messages or 60 s ("catching up").
//! - 401/403 stops everything; 429/5xx/network stops the run keeping marks;
//!   409 disconnects one account; a 400 batch is re-posted one by one.
//!
//! Privacy: only counts, states and error classes leave this module.

use std::collections::{BTreeMap, HashMap};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::Mutex;
use std::time::Duration;

use serde::Serialize;

use super::flow_client::{
    messages_body_len, AccountRegistration, ApiError, ConnectionStatus, FlowApi, HeartbeatStatus, HttpFlowApi,
    PostResult, RegisteredAccount, ALREADY_CONNECTED_DIRECTLY,
};
use super::jxa::OsascriptRunner;
use super::parse::{clamp_str, iso_to_ms, to_connector_message, ConnectorMessage, MAX_MAILBOX};
use super::state::{
    load_marks, read_start, save_marks, ConnectorConfig, Cursor, MailConnectorState, Marks, SelectedAccount,
    OVERLAP_MS,
};
use super::{included_mailboxes, is_direct_account, MailAccount, ScriptError, ScriptRunner};

/// Messages per read script call.
pub const CHUNK_LIMIT: usize = 200;
/// Per account per run.
pub const ACCOUNT_MESSAGE_CAP: usize = 2_000;
/// Per account per run, so one catching-up account can't starve the others.
pub const ACCOUNT_TIME_CAP_MS: i64 = 60_000;
/// In-script time budget per read call; keeps each osascript run short.
pub const SCRIPT_BUDGET_MS: u64 = 20_000;
/// Breathing room for Mail between osascript calls.
pub const SCRIPT_PAUSE_MS: u64 = 250;
/// FLOW takes ≤100 messages per `POST /messages`.
pub const BATCH_MAX_MESSAGES: usize = 100;
/// FLOW 413s bodies over 10 MB; start a new batch before 8 MiB.
pub const BATCH_MAX_BYTES: usize = 8 * 1024 * 1024;
/// FLOW's `POST /accounts` limits.
pub const MAX_ADDRESSES: usize = 50;
pub const MAX_DISPLAY_NAME: usize = 200;
/// First run shortly after launch (let the app settle), then every 5 min
/// after each run finishes.
pub const FIRST_RUN_DELAY: Duration = Duration::from_secs(15);
pub const RUN_INTERVAL: Duration = Duration::from_secs(300);

/// Time, injectable so tests control it and the 250 ms pauses cost nothing.
pub trait Clock {
    fn now_ms(&self) -> i64;
    fn sleep(&self, ms: u64);
}

pub struct SystemClock;

impl Clock for SystemClock {
    fn now_ms(&self) -> i64 {
        super::state::now_ms()
    }
    fn sleep(&self, ms: u64) {
        std::thread::sleep(Duration::from_millis(ms));
    }
}

/// The connector's headline state, as the UI's status dot reads it.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Overall {
    /// No key, no FLOW URL, or no ticked accounts. Nothing is attempted.
    NotConfigured,
    /// Configured, but no run has finished yet this session.
    Idle,
    Ok,
    MailNotRunning,
    PermissionDenied,
    KeyRejected,
    FlowUnavailable,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum AccountState {
    /// Not read yet this session.
    Pending,
    Ok,
    /// The last run stopped on the 2,000-message / 60 s limit with mail left.
    CatchingUp,
    /// FLOW says this connection is disconnected (409 or `status`).
    Disconnected,
    /// No valid email address, so it can't be registered.
    NoAddress,
    /// A productionpoint.co.uk account: FLOW already reads it directly, so
    /// the helper never registers or reads it.
    Direct,
    Error,
}

/// What a run needs, copied out of the config (so the run never holds the
/// config lock) plus the marks file.
#[derive(Debug, Clone)]
pub struct RunInput {
    pub has_key: bool,
    pub flow_url: Option<String>,
    pub selected: Vec<SelectedAccount>,
    pub marks: Marks,
}

/// One account's outcome. `None` fields mean "unchanged by this run".
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AccountRun {
    pub id: String,
    pub address: String,
    pub state: Option<AccountState>,
    pub connection_id: Option<String>,
    pub read: u32,
    pub posted: u32,
    pub duplicates: u32,
    pub skipped: u32,
    pub last_read_at: Option<i64>,
    /// Today's total after this run (reset at local midnight).
    pub sent_today: u32,
    pub sent_today_date: String,
    /// Error class for the `error` heartbeat; never content.
    detail: Option<String>,
}

#[derive(Debug, Clone)]
pub struct RunSummary {
    pub overall: Overall,
    pub per_account: Vec<AccountRun>,
    /// The marks after this run, to be saved.
    pub marks: Marks,
    pub finished_at: i64,
}

/// One tick. See the module docs for the rules.
pub fn run_once(
    input: RunInput,
    script: &dyn ScriptRunner,
    api: &dyn FlowApi,
    clock: &dyn Clock,
) -> RunSummary {
    let now = clock.now_ms();
    let today = local_date(now);
    let mut run = Run {
        script,
        api,
        clock,
        now,
        marks: input.marks,
        accounts: input.selected.iter().map(|s| AccountRun::start(s, &today)).collect(),
    };

    if !input.has_key || input.flow_url.is_none() || input.selected.is_empty() {
        return run.finish(Overall::NotConfigured);
    }

    // 1. Is Mail running, and may we talk to it? (The only script call
    //    without a pause before it.)
    if let Err(e) = super::ping(script) {
        let cached: Vec<String> = input
            .selected
            .iter()
            .filter(|s| !is_direct_account(&s.account))
            .filter_map(|s| s.connection_id.clone())
            .collect();
        let overall = run.mail_unavailable(&e, &cached);
        return run.finish(overall);
    }

    // 2. Register every selected account that has a valid address and that
    //    FLOW doesn't already read directly (productionpoint.co.uk).
    let mut regs: Vec<(usize, AccountRegistration)> = Vec::new();
    for (i, s) in input.selected.iter().enumerate() {
        if is_direct_account(&s.account) {
            run.accounts[i].state = Some(AccountState::Direct);
            continue;
        }
        match registration(&s.account) {
            Some(r) => regs.push((i, r)),
            None => run.accounts[i].state = Some(AccountState::NoAddress),
        }
    }
    if regs.is_empty() {
        return run.finish(Overall::Ok);
    }
    let bodies: Vec<AccountRegistration> = regs.iter().map(|(_, r)| r.clone()).collect();
    let registered: Vec<RegisteredAccount> = match api.post_accounts(&bodies) {
        Ok(r) => r.accounts,
        Err(ApiError::Conflict) => {
            // FLOW 409s the whole post only when it refuses every account.
            // Re-post each alone (also covers an older FLOW that refused the
            // whole post for one account), so one refusal can't block the
            // rest.
            let mut all = Vec::new();
            for (i, reg) in &regs {
                match api.post_accounts(std::slice::from_ref(reg)) {
                    Ok(r) => all.extend(r.accounts),
                    Err(ApiError::Conflict) => {
                        run.accounts[*i].state = Some(refused_state(&input.selected[*i].account, None));
                    }
                    Err(e @ (ApiError::KeyRejected | ApiError::Unavailable)) => return run.finish(run_stop_for(&e)),
                    Err(_) => {
                        run.accounts[*i].state = Some(AccountState::Error);
                        run.accounts[*i].detail = Some("registration failed".into());
                    }
                }
            }
            all
        }
        Err(e) => return run.finish(run_stop_for(&e)),
    };

    // FLOW lower-cases addresses; match back the same way. Each item stands
    // alone: a refused or odd one only affects its own account.
    let mut targets: Vec<Target> = Vec::new();
    for (i, reg) in &regs {
        if run.accounts[*i].state.is_some() {
            continue; // settled by the one-by-one re-post
        }
        let want = reg.address.to_lowercase();
        let account = &input.selected[*i].account;
        match registered.iter().find(|r| r.address.to_lowercase() == want) {
            None => {
                run.accounts[*i].state = Some(AccountState::Error);
                run.accounts[*i].detail = Some("not registered".into());
            }
            Some(r) => match (r.status, &r.connection_id) {
                (ConnectionStatus::Connected, Some(conn)) => {
                    run.accounts[*i].connection_id = Some(conn.clone());
                    targets.push(Target {
                        index: *i,
                        account_id: account.apple_account_id.clone(),
                        connection_id: conn.clone(),
                        resume_from: r.resume_from.clone(),
                    });
                }
                (ConnectionStatus::Disconnected, Some(conn)) => {
                    run.accounts[*i].connection_id = Some(conn.clone());
                    run.accounts[*i].state = Some(AccountState::Disconnected);
                }
                // Refused, an unknown status, or no connectionId.
                _ => {
                    run.accounts[*i].state = Some(refused_state(account, r.error.as_deref()));
                    run.accounts[*i].detail = Some("refused".into());
                }
            },
        }
    }

    // 3. Read and post, account by account.
    for (n, t) in targets.iter().enumerate() {
        let outcome = run.read_account(t);
        let account = &run.accounts[t.index];
        let beat = match (&outcome, account.state) {
            (Err(Halt::Run(o @ (Overall::MailNotRunning | Overall::PermissionDenied))), _) => {
                // Mail went away mid-run: tell FLOW for this and every
                // account we didn't get to.
                let conns: Vec<String> = targets[n..].iter().map(|t| t.connection_id.clone()).collect();
                let overall = run.heartbeat_all(&conns, mail_heartbeat(*o), *o);
                return run.finish(overall);
            }
            (Err(Halt::Run(o)), _) => return run.finish(*o),
            (_, Some(AccountState::Error)) => Some((HeartbeatStatus::Error, account.detail.clone())),
            (_, Some(AccountState::Ok | AccountState::CatchingUp)) => Some((HeartbeatStatus::Ok, None)),
            _ => None,
        };
        if let Some((status, detail)) = beat {
            match api.heartbeat(&t.connection_id, status, detail.as_deref()) {
                Ok(()) => {}
                Err(ApiError::Conflict) => run.accounts[t.index].state = Some(AccountState::Disconnected),
                Err(e @ (ApiError::KeyRejected | ApiError::Unavailable)) => return run.finish(run_stop_for(&e)),
                Err(_) => {}
            }
        }
    }

    let any_error = run.accounts.iter().any(|a| a.state == Some(AccountState::Error));
    run.finish(if any_error { Overall::Error } else { Overall::Ok })
}

/// A connected account to read this run.
struct Target {
    index: usize,
    account_id: String,
    connection_id: String,
    resume_from: Option<BTreeMap<String, String>>,
}

/// Where a mailbox's reads start this run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Start {
    pub since: i64,
    pub after_id: Option<i64>,
    /// The cursor to store before reading (kept if nothing posts).
    pub stored: Cursor,
}

/// The per-mailbox start rule (review round 1, I2 ruling):
/// - (a) FLOW's `resumeFrom` is null → now − 60 d, no afterId (reset).
/// - (b) the local cursor is catching up and is no further back than FLOW's
///   mark − 10 min (or FLOW has no key for the mailbox) → resume exactly at
///   the local compound cursor, no overlap.
/// - (c) otherwise → FLOW's mark − 10 min (now − 60 d with no key), no
///   afterId.
///
/// (b) accepts a cursor up to the overlap *behind* FLOW's mark, not only at
/// or past it: after a finished run the next tick re-reads FLOW's mark −
/// 10 min, and if that window holds more than one tick can read the cursor
/// ends inside it, behind FLOW's mark (everything re-read was a duplicate, so
/// the mark doesn't move). Requiring `ms ≥ mark` would restart that same
/// window every tick and never reach new mail.
pub fn start_cursor(
    resume_from: Option<&BTreeMap<String, String>>,
    posted_as: &str,
    local: Option<Cursor>,
    now: i64,
) -> Start {
    let Some(map) = resume_from else {
        let since = read_start(None, now);
        return Start { since, after_id: None, stored: Cursor::at(since) };
    };
    let flow = map.get(posted_as).and_then(|iso| iso_to_ms(iso));
    if let Some(c) = local.filter(|c| c.catching_up && flow.is_none_or(|f| c.ms >= f - OVERLAP_MS)) {
        return Start { since: c.ms, after_id: c.after_id, stored: c };
    }
    let since = read_start(flow, now);
    Start { since, after_id: None, stored: Cursor::at(flow.unwrap_or(since)) }
}

/// Why reading an account stopped early.
enum Halt {
    /// Stop the whole run with this overall state.
    Run(Overall),
    /// Stop this account only; its state says why.
    Account,
}

/// The overall state for a FLOW error that ends the run.
fn run_stop_for(e: &ApiError) -> Overall {
    match e {
        ApiError::KeyRejected => Overall::KeyRejected,
        ApiError::Unavailable => Overall::FlowUnavailable,
        _ => Overall::Error,
    }
}

fn mail_heartbeat(o: Overall) -> HeartbeatStatus {
    if o == Overall::PermissionDenied {
        HeartbeatStatus::PermissionDenied
    } else {
        HeartbeatStatus::MailNotRunning
    }
}

/// One run's working state.
struct Run<'a> {
    script: &'a dyn ScriptRunner,
    api: &'a dyn FlowApi,
    clock: &'a dyn Clock,
    /// When the run started; the 60-day lookback is measured from here.
    now: i64,
    marks: Marks,
    accounts: Vec<AccountRun>,
}

impl Run<'_> {
    fn finish(self, overall: Overall) -> RunSummary {
        RunSummary { overall, per_account: self.accounts, marks: self.marks, finished_at: self.clock.now_ms() }
    }

    /// Every osascript call after the ping goes through here, so Mail gets a
    /// breather between them.
    fn script<T>(&self, f: impl FnOnce(&dyn ScriptRunner) -> Result<T, ScriptError>) -> Result<T, ScriptError> {
        self.clock.sleep(SCRIPT_PAUSE_MS);
        f(self.script)
    }

    /// Mail isn't running / permission denied: heartbeat the connections and
    /// return the overall state. Other script failures are an `error`.
    fn mail_unavailable(&self, e: &ScriptError, conns: &[String]) -> Overall {
        match e {
            ScriptError::NotRunning => self.heartbeat_all(conns, HeartbeatStatus::MailNotRunning, Overall::MailNotRunning),
            ScriptError::PermissionDenied => {
                self.heartbeat_all(conns, HeartbeatStatus::PermissionDenied, Overall::PermissionDenied)
            }
            ScriptError::Other(class) => {
                for c in conns {
                    let _ = self.api.heartbeat(c, HeartbeatStatus::Error, Some(&clamp_str(class, 500)));
                }
                Overall::Error
            }
        }
    }

    /// Send `status` for each connection. A rejected key outranks `overall`;
    /// any other heartbeat failure doesn't change what the user must do.
    fn heartbeat_all(&self, conns: &[String], status: HeartbeatStatus, overall: Overall) -> Overall {
        for c in conns {
            if let Err(ApiError::KeyRejected) = self.api.heartbeat(c, status, None) {
                return Overall::KeyRejected;
            }
        }
        overall
    }

    /// Map a script failure inside an account's reads.
    fn script_failed(&mut self, index: usize, e: ScriptError) -> Halt {
        match e {
            ScriptError::NotRunning => Halt::Run(Overall::MailNotRunning),
            ScriptError::PermissionDenied => Halt::Run(Overall::PermissionDenied),
            ScriptError::Other(class) => {
                let a = &mut self.accounts[index];
                a.state = Some(AccountState::Error);
                a.detail = Some(clamp_str(&class, 500));
                Halt::Account
            }
        }
    }

    /// Map a FLOW failure on `/messages`.
    fn post_failed(&mut self, index: usize, e: ApiError) -> Halt {
        match e {
            ApiError::Conflict => {
                self.accounts[index].state = Some(AccountState::Disconnected);
                Halt::Account
            }
            ApiError::KeyRejected | ApiError::Unavailable => Halt::Run(run_stop_for(&e)),
            ApiError::BadRequest | ApiError::TooLarge => {
                // Only reached for a single message that still fails, which
                // the caller counts as skipped; listed for exhaustiveness.
                Halt::Account
            }
            ApiError::Other(class) => {
                let a = &mut self.accounts[index];
                a.state = Some(AccountState::Error);
                a.detail = Some(format!("flow {class}"));
                Halt::Account
            }
        }
    }

    fn read_account(&mut self, t: &Target) -> Result<(), Halt> {
        let started = self.clock.now_ms();
        let mut read_total = 0usize;

        let paths = match self.script(|r| super::list_mailboxes(r, &t.account_id)) {
            Ok(p) => p,
            Err(e) => return Err(self.script_failed(t.index, e)),
        };
        let included = included_mailboxes(&paths);

        // FLOW's marks win on every registration: rebuild this account's
        // marks from resumeFrom (keyed by the path as we post it); a missing
        // key or a null map starts 60 days back. This also drops marks of
        // mailboxes that no longer exist.
        let mut local = self.marks.take_account(&t.account_id);
        let mut starts = Vec::with_capacity(included.len());
        for mb in &included {
            let posted_as = clamp_str(mb, MAX_MAILBOX);
            let cursor = start_cursor(t.resume_from.as_ref(), &posted_as, local.remove(mb), self.now);
            self.marks.set_cursor(&t.account_id, mb, cursor.stored);
            starts.push((mb.clone(), posted_as, cursor.since, cursor.after_id));
        }

        for (mb, posted_as, mut since, mut after_id) in starts {
            loop {
                if read_total >= ACCOUNT_MESSAGE_CAP || self.clock.now_ms() - started >= ACCOUNT_TIME_CAP_MS {
                    // Out of this run's allowance with mail (possibly) left:
                    // the next tick resumes from FLOW's resumeFrom, which is
                    // where these posts left it.
                    self.accounts[t.index].state = Some(AccountState::CatchingUp);
                    return Ok(());
                }
                let limit = CHUNK_LIMIT.min(ACCOUNT_MESSAGE_CAP - read_total);
                let chunk = match self.script(|r| {
                    super::read_mailbox_since(r, &t.account_id, &mb, since, after_id, limit, SCRIPT_BUDGET_MS)
                }) {
                    Ok(c) => c,
                    Err(e) => return Err(self.script_failed(t.index, e)),
                };
                let a = &mut self.accounts[t.index];
                a.last_read_at = Some(self.clock.now_ms());
                read_total += chunk.messages.len();
                a.read += chunk.messages.len() as u32;

                // The script sorts by (date, id); take the max anyway.
                if let Some(last) = chunk.messages.iter().map(|m| (m.date_received, m.mail_id)).max() {
                    let messages: Vec<ConnectorMessage> =
                        chunk.messages.iter().filter_map(to_connector_message).collect();
                    self.accounts[t.index].skipped += (chunk.messages.len() - messages.len()) as u32;
                    self.post_chunk(t, &posted_as, messages)?;
                    // Every batch posted: advance past the whole chunk
                    // (≥ FLOW's highWater, and past skipped messages). The
                    // compound cursor resumes inside a same-second group.
                    (since, after_id) = last;
                    let cursor = Cursor { ms: since, after_id, catching_up: chunk.more };
                    self.marks.set_cursor(&t.account_id, &mb, cursor);
                } else if !chunk.more {
                    // Read to the end with nothing new: no longer catching up.
                    if let Some(c) = self.marks.cursor(&t.account_id, &mb) {
                        self.marks.set_cursor(&t.account_id, &mb, Cursor { catching_up: false, ..c });
                    }
                }
                if chunk.not_running {
                    return Err(Halt::Run(Overall::MailNotRunning));
                }
                if !chunk.more || chunk.messages.is_empty() {
                    break;
                }
            }
        }
        self.accounts[t.index].state = Some(AccountState::Ok);
        Ok(())
    }

    /// Post one chunk in batches. A 400/413 batch is re-posted one message at
    /// a time and the ones that still fail are skipped, so one bad message
    /// can't wedge a mailbox.
    fn post_chunk(&mut self, t: &Target, mailbox: &str, messages: Vec<ConnectorMessage>) -> Result<(), Halt> {
        for batch in split_batches(&t.connection_id, mailbox, messages, BATCH_MAX_BYTES) {
            match self.api.post_messages(&t.connection_id, mailbox, &batch) {
                Ok(r) => self.accounts[t.index].count(&r),
                // Already alone: re-posting it would just fail again.
                Err(ApiError::BadRequest | ApiError::TooLarge) if batch.len() == 1 => {
                    self.accounts[t.index].skipped += 1
                }
                Err(ApiError::BadRequest | ApiError::TooLarge) => {
                    for m in &batch {
                        match self.api.post_messages(&t.connection_id, mailbox, std::slice::from_ref(m)) {
                            Ok(r) => self.accounts[t.index].count(&r),
                            Err(ApiError::BadRequest | ApiError::TooLarge) => self.accounts[t.index].skipped += 1,
                            Err(e) => return Err(self.post_failed(t.index, e)),
                        }
                    }
                }
                Err(e) => return Err(self.post_failed(t.index, e)),
            }
        }
        Ok(())
    }
}

impl AccountRun {
    fn start(s: &SelectedAccount, today: &str) -> Self {
        AccountRun {
            id: s.account.apple_account_id.clone(),
            address: display_address(&s.account),
            state: None,
            connection_id: None,
            read: 0,
            posted: 0,
            duplicates: 0,
            skipped: 0,
            last_read_at: None,
            sent_today: sent_today(s, today),
            sent_today_date: today.to_string(),
            detail: None,
        }
    }

    fn count(&mut self, r: &PostResult) {
        self.posted += r.accepted;
        self.duplicates += r.duplicates;
        self.sent_today += r.accepted;
    }
}

/// Today's count, or 0 once the stored date isn't today.
fn sent_today(s: &SelectedAccount, today: &str) -> u32 {
    if s.sent_today_date.as_deref() == Some(today) {
        s.sent_today
    } else {
        0
    }
}

/// State for an account FLOW refused (per item, or by a 409 on its own).
pub fn refused_state(account: &MailAccount, error: Option<&str>) -> AccountState {
    if error == Some(ALREADY_CONNECTED_DIRECTLY) || is_direct_account(account) {
        AccountState::Direct
    } else {
        AccountState::Error
    }
}

/// The address to show: the one we register, else whatever Mail has.
fn display_address(a: &MailAccount) -> String {
    registration(a)
        .map(|r| r.address)
        .or_else(|| a.addresses.iter().map(|x| x.trim()).find(|x| !x.is_empty()).map(String::from))
        .unwrap_or_default()
}

/// `POST /accounts` entry for a Mail account, or `None` when it has no valid
/// address. Invalid aliases are dropped: one bad alias 400s the whole post.
pub fn registration(account: &MailAccount) -> Option<AccountRegistration> {
    let mut addresses: Vec<String> = Vec::new();
    for a in &account.addresses {
        let a = a.trim();
        if is_valid_email(a) && !addresses.iter().any(|x| x.eq_ignore_ascii_case(a)) {
            addresses.push(a.to_string());
        }
    }
    addresses.truncate(MAX_ADDRESSES);
    let address = addresses.first()?.clone();
    let name = clamp_str(account.name.trim(), MAX_DISPLAY_NAME);
    Some(AccountRegistration {
        display_name: if name.trim().is_empty() { address.clone() } else { name },
        address,
        addresses,
        apple_account_id: account.apple_account_id.clone(),
    })
}

/// A conservative email check, modelled on zod's `.email()` (what FLOW's
/// validation uses): local part of `A-Z a-z 0-9 _ ' + - .` (no leading,
/// trailing or doubled dot), a domain of dot-separated `[A-Za-z0-9-]` labels
/// not starting with `-`, and an alphabetic TLD of ≥2. Stricter than the
/// server only drops an alias; looser would 400 every registration.
fn is_valid_email(s: &str) -> bool {
    if s.len() > 254 {
        return false;
    }
    let Some((local, domain)) = s.split_once('@') else {
        return false;
    };
    let local_ok = !local.is_empty()
        && !local.starts_with('.')
        && !local.ends_with('.')
        && !local.contains("..")
        && local.chars().all(|c| c.is_ascii_alphanumeric() || "_'+-.".contains(c));
    let labels: Vec<&str> = domain.split('.').collect();
    let domain_ok = labels.len() >= 2
        && labels.iter().all(|l| {
            !l.is_empty() && !l.starts_with('-') && l.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        && labels.last().is_some_and(|tld| tld.len() >= 2 && tld.chars().all(|c| c.is_ascii_alphabetic()));
    local_ok && domain_ok
}

/// Split a chunk into `POST /messages` bodies of ≤100 messages and at most
/// `max_bytes` serialised. A single message is ≤ ~150 KB after clamping, so a
/// batch always holds at least one.
pub fn split_batches(
    connection_id: &str,
    mailbox: &str,
    messages: Vec<ConnectorMessage>,
    max_bytes: usize,
) -> Vec<Vec<ConnectorMessage>> {
    // Body = envelope + messages joined by commas; track it exactly.
    let envelope = messages_body_len(connection_id, mailbox, &[]);
    let mut batches = Vec::new();
    let mut current: Vec<ConnectorMessage> = Vec::new();
    let mut bytes = envelope;
    for m in messages {
        let len = serde_json::to_vec(&m).map(|v| v.len()).unwrap_or(usize::MAX / 4);
        let added = if current.is_empty() { len } else { len + 1 };
        if !current.is_empty() && (current.len() >= BATCH_MAX_MESSAGES || bytes + added > max_bytes) {
            batches.push(std::mem::take(&mut current));
            bytes = envelope;
        }
        bytes += if current.is_empty() { len } else { len + 1 };
        current.push(m);
    }
    if !current.is_empty() {
        batches.push(current);
    }
    batches
}

/// Local calendar date (`YYYY-MM-DD`) of an epoch-ms instant; `sentToday`
/// resets when it changes.
pub fn local_date(ms: i64) -> String {
    use chrono::TimeZone;
    chrono::Local
        .timestamp_millis_opt(ms)
        .single()
        .map(|d| d.format("%Y-%m-%d").to_string())
        .unwrap_or_default()
}

// ── Shared status ───────────────────────────────────────────────────────────

/// `mail_connector_status`'s result — the shape the Settings UI consumes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConnectorStatus {
    pub running: bool,
    pub last_run_at: Option<i64>,
    pub overall: Overall,
    pub accounts: Vec<AccountStatus>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AccountStatus {
    pub id: String,
    pub name: String,
    pub address: String,
    pub state: AccountState,
    pub last_read_at: Option<i64>,
    pub sent_today: u32,
}

/// In-memory state of the loop since launch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RuntimeSnapshot {
    pub running: bool,
    pub last_run_at: Option<i64>,
    pub overall: Option<Overall>,
    pub states: HashMap<String, AccountState>,
}

/// Tauri-managed: the status the loop publishes and the loop's wake channel.
#[derive(Default)]
pub struct ConnectorRuntime {
    inner: Mutex<RuntimeSnapshot>,
    waker: Mutex<Option<SyncSender<()>>>,
}

impl ConnectorRuntime {
    pub fn snapshot(&self) -> RuntimeSnapshot {
        lock(&self.inner).clone()
    }

    /// Ask the loop to run now. The channel holds one pending wake, so any
    /// number of wakes while a run is in flight coalesce into a single
    /// follow-up run (a "Connect" during a scheduled run still gets read
    /// straight after it). Returns whether this call queued a wake.
    pub fn wake(&self) -> bool {
        match lock(&self.waker).as_ref() {
            Some(tx) => match tx.try_send(()) {
                Ok(()) => true,
                Err(TrySendError::Full(())) | Err(TrySendError::Disconnected(())) => false,
            },
            None => false,
        }
    }

    fn set_waker(&self, tx: SyncSender<()>) {
        *lock(&self.waker) = Some(tx);
    }

    fn set_running(&self, running: bool) {
        lock(&self.inner).running = running;
    }

    fn record(&self, summary: &RunSummary) {
        let mut inner = lock(&self.inner);
        inner.last_run_at = Some(summary.finished_at);
        inner.overall = Some(summary.overall);
        for a in &summary.per_account {
            if let Some(state) = a.state {
                inner.states.insert(a.id.clone(), state);
            }
        }
    }
}

/// The status is only ever overwritten whole, so a panic mid-update can't
/// leave it inconsistent: recover from poisoning rather than going dark.
fn lock<T>(m: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    m.lock().unwrap_or_else(|e| e.into_inner())
}

/// Combine the config (persisted bookkeeping), the loop's in-memory state and
/// whether a key exists into the UI's status.
pub fn build_status(
    config: &ConnectorConfig,
    rt: &RuntimeSnapshot,
    has_key: bool,
    now_ms: i64,
) -> ConnectorStatus {
    let configured = has_key && config.flow_url.is_some() && !config.selected.is_empty();
    let overall = match rt.overall {
        _ if !configured => Overall::NotConfigured,
        // Configured since that run: nothing has happened yet.
        None | Some(Overall::NotConfigured) => Overall::Idle,
        Some(o) => o,
    };
    let today = local_date(now_ms);
    let accounts = config
        .selected
        .iter()
        .map(|s| {
            let id = &s.account.apple_account_id;
            let state = if is_direct_account(&s.account) {
                AccountState::Direct
            } else if registration(&s.account).is_none() {
                AccountState::NoAddress
            } else {
                match rt.states.get(id) {
                    // Metadata was refreshed since; it's registrable now.
                    None | Some(AccountState::NoAddress) => AccountState::Pending,
                    Some(st) => *st,
                }
            };
            AccountStatus {
                id: id.clone(),
                name: s.account.name.clone(),
                address: display_address(&s.account),
                state,
                last_read_at: s.last_read_at,
                sent_today: sent_today(s, &today),
            }
        })
        .collect();
    ConnectorStatus { running: rt.running, last_run_at: rt.last_run_at, overall, accounts }
}

/// Write a run's per-account bookkeeping into the config. Matches by account
/// id against the *current* selection, so a change the user made mid-run
/// isn't overwritten.
pub fn apply_summary(config: &mut ConnectorConfig, summary: &RunSummary) {
    for s in &mut config.selected {
        let Some(r) = summary.per_account.iter().find(|r| r.id == s.account.apple_account_id) else {
            continue;
        };
        if let Some(c) = &r.connection_id {
            s.connection_id = Some(c.clone());
        }
        if let Some(t) = r.last_read_at {
            s.last_read_at = Some(t);
        }
        s.sent_today = r.sent_today;
        s.sent_today_date = Some(r.sent_today_date.clone());
    }
}

/// The loop body: wait (first `first`, then `every` after each run) or until
/// woken, run, repeat; returns when the channel's sender is gone. One thread
/// runs this, so runs are single-flight by construction.
fn drive(rx: &Receiver<()>, first: Duration, every: Duration, mut run: impl FnMut()) {
    let mut wait = first;
    loop {
        match rx.recv_timeout(wait) {
            Ok(()) | Err(RecvTimeoutError::Timeout) => run(),
            Err(RecvTimeoutError::Disconnected) => return,
        }
        wait = every;
    }
}

/// Start the background loop (macOS only; called from `setup`). The first
/// run is ~15 s after launch, then every 5 min after each run finishes, or
/// sooner when `mail_connector_run_now` wakes it.
// Off macOS nothing calls this (there's no Apple Mail), but it and what it
// reaches still compile there, so don't flag them as dead.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
pub fn start_loop(app: tauri::AppHandle) {
    use tauri::Manager;
    let Some(rt) = app.try_state::<ConnectorRuntime>() else {
        return;
    };
    let (tx, rx) = std::sync::mpsc::sync_channel(1);
    rt.set_waker(tx);
    let thread_app = app.clone();
    let spawned = std::thread::Builder::new()
        .name("mail-connector".into())
        .spawn(move || drive(&rx, FIRST_RUN_DELAY, RUN_INTERVAL, || tick(&thread_app)));
    if spawned.is_err() {
        *lock(&rt.waker) = None;
    }
}

/// One real run: real Mail, real FLOW, real clock; then persist.
fn tick(app: &tauri::AppHandle) {
    use tauri::Manager;
    let (Some(state), Some(rt)) = (app.try_state::<MailConnectorState>(), app.try_state::<ConnectorRuntime>()) else {
        return;
    };
    rt.set_running(true);
    // A panic must not kill the loop thread (the connector would silently
    // stop until relaunch). Our code never panics with content in the
    // message; the run just reports `error`.
    let summary = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| real_run(&state))).unwrap_or_else(|_| {
        let config = state.config();
        failed_run(Overall::Error, &config.selected, load_marks(&state.marks_path()))
    });
    // Marks first: they're the expensive thing to lose. FLOW dedupes, so a
    // failed write only means re-reading next tick.
    let _ = save_marks(&state.marks_path(), &summary.marks);
    let _ = state.update_config(|c| {
        apply_summary(c, &summary);
        Ok(())
    });
    rt.record(&summary);
    rt.set_running(false);
}

fn real_run(state: &MailConnectorState) -> RunSummary {
    let config = state.config();
    let marks = load_marks(&state.marks_path());
    let key = match crate::commands::get_mail_connector_key() {
        Ok(k) => k,
        Err(_) => return failed_run(Overall::Error, &config.selected, marks),
    };
    let (Some(key), Some(url)) = (key, config.flow_url.clone()) else {
        return failed_run(Overall::NotConfigured, &config.selected, marks);
    };
    let api = match HttpFlowApi::new(&url, &key) {
        Ok(api) => api,
        Err(_) => return failed_run(Overall::Error, &config.selected, marks),
    };
    let input = RunInput { has_key: true, flow_url: Some(url), selected: config.selected, marks };
    run_once(input, &OsascriptRunner, &api, &SystemClock)
}

/// A run that stopped before doing anything.
fn failed_run(overall: Overall, selected: &[SelectedAccount], marks: Marks) -> RunSummary {
    let now = super::state::now_ms();
    let today = local_date(now);
    RunSummary {
        overall,
        per_account: selected.iter().map(|s| AccountRun::start(s, &today)).collect(),
        marks,
        finished_at: now,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mail_connector::flow_client::{AccountsResponse, RegisteredAccount};
    use crate::mail_connector::state::INITIAL_LOOKBACK_MS;
    use crate::mail_connector::tests::{params_of, FakeRunner};
    use crate::mail_connector::jxa;
    use std::cell::{Cell, RefCell};

    const NOW: i64 = 1_761_472_800_000; // 2025-10-26T10:00:00.000Z
    const DAY: i64 = 86_400_000;

    // ── fakes ──

    struct FakeClock {
        now: Cell<i64>,
        slept: Cell<u64>,
    }
    impl FakeClock {
        fn new() -> Self {
            FakeClock { now: Cell::new(NOW), slept: Cell::new(0) }
        }
        fn advance(&self, ms: i64) {
            self.now.set(self.now.get() + ms);
        }
    }
    impl Clock for FakeClock {
        fn now_ms(&self) -> i64 {
            self.now.get()
        }
        fn sleep(&self, ms: u64) {
            self.slept.set(self.slept.get() + ms);
            self.advance(ms as i64);
        }
    }

    /// FakeRunner whose every script call costs `cost_ms` of fake time.
    struct SlowRunner<'a> {
        inner: &'a FakeRunner,
        clock: &'a FakeClock,
        cost_ms: i64,
    }
    impl ScriptRunner for SlowRunner<'_> {
        fn run_jxa(&self, script: &str) -> Result<String, ScriptError> {
            self.clock.advance(self.cost_ms);
            self.inner.run_jxa(script)
        }
    }

    type MessagesFn = Box<dyn Fn(&str, &str, &[ConnectorMessage]) -> Result<PostResult, ApiError>>;

    #[derive(Debug, Clone)]
    struct Post {
        conn: String,
        mailbox: String,
        ids: Vec<String>,
        bytes: usize,
    }

    type AccountsFn = Box<dyn Fn(&[AccountRegistration]) -> Result<AccountsResponse, ApiError>>;

    struct FakeApi {
        on_accounts: AccountsFn,
        on_messages: MessagesFn,
        heartbeat: Result<(), ApiError>,
        registered: RefCell<Vec<Vec<AccountRegistration>>>,
        posts: RefCell<Vec<Post>>,
        beats: RefCell<Vec<(String, HeartbeatStatus, Option<String>)>>,
    }
    impl FakeApi {
        fn new(accounts: Vec<RegisteredAccount>) -> Self {
            FakeApi {
                on_accounts: Box::new(move |_| Ok(AccountsResponse { accounts: accounts.clone() })),
                on_messages: Box::new(|_, _, m| Ok(PostResult { accepted: m.len() as u32, duplicates: 0, high_water: None })),
                heartbeat: Ok(()),
                registered: RefCell::new(Vec::new()),
                posts: RefCell::new(Vec::new()),
                beats: RefCell::new(Vec::new()),
            }
        }
        fn on_messages(mut self, f: impl Fn(&str, &str, &[ConnectorMessage]) -> Result<PostResult, ApiError> + 'static) -> Self {
            self.on_messages = Box::new(f);
            self
        }
        fn calls(&self) -> usize {
            self.registered.borrow().len() + self.posts.borrow().len() + self.beats.borrow().len()
        }
        fn posted_ids(&self) -> Vec<String> {
            self.posts.borrow().iter().flat_map(|p| p.ids.clone()).collect()
        }
    }
    impl FlowApi for FakeApi {
        fn post_accounts(&self, accounts: &[AccountRegistration]) -> Result<AccountsResponse, ApiError> {
            self.registered.borrow_mut().push(accounts.to_vec());
            (self.on_accounts)(accounts)
        }
        fn post_messages(&self, conn: &str, mailbox: &str, messages: &[ConnectorMessage]) -> Result<PostResult, ApiError> {
            self.posts.borrow_mut().push(Post {
                conn: conn.into(),
                mailbox: mailbox.into(),
                ids: messages.iter().map(|m| m.internet_message_id.clone()).collect(),
                bytes: crate::mail_connector::flow_client::messages_body_len(conn, mailbox, messages),
            });
            (self.on_messages)(conn, mailbox, messages)
        }
        fn heartbeat(&self, conn: &str, status: HeartbeatStatus, detail: Option<&str>) -> Result<(), ApiError> {
            self.beats.borrow_mut().push((conn.into(), status, detail.map(String::from)));
            self.heartbeat.clone()
        }
    }

    // ── builders ──

    fn account(id: &str, addresses: &[&str]) -> SelectedAccount {
        SelectedAccount::new(MailAccount {
            apple_account_id: id.into(),
            name: format!("Account {id}"),
            addresses: addresses.iter().map(|s| s.to_string()).collect(),
            kind: "imap".into(),
        })
    }

    fn input(selected: Vec<SelectedAccount>) -> RunInput {
        RunInput { has_key: true, flow_url: Some("https://flow.test".into()), selected, marks: Marks::default() }
    }

    fn connected(address: &str, conn: &str, resume: Option<&[(&str, &str)]>) -> RegisteredAccount {
        RegisteredAccount {
            address: address.into(),
            connection_id: Some(conn.into()),
            status: ConnectionStatus::Connected,
            error: None,
            resume_from: resume.map(|r| r.iter().map(|(k, v)| (k.to_string(), v.to_string())).collect::<BTreeMap<_, _>>()),
        }
    }

    fn ping_ok() -> Result<String, ScriptError> {
        Ok(r#"{"ok":true}"#.into())
    }

    fn mailboxes(paths: &[&str]) -> Result<String, ScriptError> {
        Ok(serde_json::json!({ "ok": paths }).to_string())
    }

    fn msg(id: &str, date: i64, body_len: usize) -> serde_json::Value {
        serde_json::json!({
            "messageId": id, "dateReceived": date, "dateSent": null, "sender": "a@x",
            "subject": "s", "to": [], "cc": [], "attachments": [], "headers": "",
            "content": "b".repeat(body_len)
        })
    }

    /// A read result of `n` messages dated `start+1 ..= start+n`.
    fn read_n(prefix: &str, start: i64, n: usize, more: bool) -> Result<String, ScriptError> {
        let msgs: Vec<_> = (1..=n).map(|i| msg(&format!("{prefix}{i}@x"), start + i as i64, 4)).collect();
        Ok(serde_json::json!({ "ok": { "messages": msgs, "more": more } }).to_string())
    }

    fn read_msgs(msgs: Vec<serde_json::Value>, more: bool) -> Result<String, ScriptError> {
        Ok(serde_json::json!({ "ok": { "messages": msgs, "more": more } }).to_string())
    }

    fn read_scripts(r: &FakeRunner) -> Vec<serde_json::Value> {
        r.scripts
            .borrow()
            .iter()
            .filter(|s| s.contains(jxa::READ_MAILBOX_BODY))
            .map(|s| params_of(s))
            .collect()
    }

    fn state_of(s: &RunSummary, id: &str) -> Option<AccountState> {
        s.per_account.iter().find(|a| a.id == id).unwrap().state
    }

    fn run(i: RunInput, r: &FakeRunner, api: &FakeApi) -> RunSummary {
        run_once(i, r, api, &FakeClock::new())
    }

    // ── brief tests ──

    #[test]
    fn http_409_marks_the_account_disconnected_and_stops_posting() {
        // "409 marks the account disconnected and stops posting" (RF 3).
        let r = FakeRunner::new(vec![
            ping_ok(),
            mailboxes(&["Inbox", "Archive"]),
            read_n("a", NOW - DAY, 3, true),
            // B is still read after A is disconnected.
            mailboxes(&["INBOX"]),
            read_n("b", NOW - DAY, 1, false),
        ]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None), connected("b@x.com", "cb", None)])
            .on_messages(|conn, _, m| {
                if conn == "ca" { Err(ApiError::Conflict) } else { Ok(PostResult { accepted: m.len() as u32, ..Default::default() }) }
            });
        let s = run(input(vec![account("A", &["a@x.com"]), account("B", &["b@x.com"])]), &r, &api);

        assert_eq!(state_of(&s, "A"), Some(AccountState::Disconnected));
        assert_eq!(state_of(&s, "B"), Some(AccountState::Ok));
        // One post for A (the 409), none after it: no second chunk, no Archive.
        let posts = api.posts.borrow();
        assert_eq!(posts.iter().filter(|p| p.conn == "ca").count(), 1);
        let reads = read_scripts(&r);
        assert_eq!(reads.iter().filter(|p| p["account"] == "A").count(), 1);
        // The mark stays where FLOW put it.
        assert_eq!(s.marks.get("A", "Inbox"), Some(NOW - INITIAL_LOOKBACK_MS));
        // No heartbeat for a disconnected connection; B gets "ok".
        assert_eq!(*api.beats.borrow(), vec![("cb".into(), HeartbeatStatus::Ok, None)]);
        assert_eq!(s.overall, Overall::Ok);
        assert_eq!(s.per_account[0].connection_id.as_deref(), Some("ca"));
    }

    #[test]
    fn catch_up_respects_run_and_batch_limits() {
        // RF 5: 60 s per account per run, batches ≤100, the other account
        // still gets its turn.
        let fake = FakeRunner::new(vec![
            ping_ok(),
            mailboxes(&["Inbox", "Archive"]),
            read_n("a", NOW - DAY, 150, true),
            read_n("b", NOW - DAY + 150, 150, true),
            read_n("c", NOW - DAY + 300, 150, true),
            // Account B.
            mailboxes(&["INBOX"]),
            read_n("e", NOW - DAY, 2, false),
        ]);
        let clock = FakeClock::new();
        let slow = SlowRunner { inner: &fake, clock: &clock, cost_ms: 15_000 };
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None), connected("b@x.com", "cb", None)]);
        let s = run_once(input(vec![account("A", &["a@x.com"]), account("B", &["b@x.com"])]), &slow, &api, &clock);

        let reads = read_scripts(&fake);
        let a_reads: Vec<_> = reads.iter().filter(|p| p["account"] == "A").collect();
        // list (15 s) + 3 reads ≈ 61 s: the 4th chunk would start past 60 s.
        assert_eq!(a_reads.len(), 3);
        assert!(a_reads.iter().all(|p| p["mailbox"] == "Inbox"), "Archive not started");
        assert_eq!(state_of(&s, "A"), Some(AccountState::CatchingUp));
        assert_eq!(state_of(&s, "B"), Some(AccountState::Ok));
        assert_eq!(reads.iter().filter(|p| p["account"] == "B").count(), 1);

        let posts = api.posts.borrow();
        assert!(posts.iter().all(|p| p.ids.len() <= BATCH_MAX_MESSAGES));
        // 150 → 100 + 50, three chunks, plus B's one batch.
        assert_eq!(posts.iter().filter(|p| p.conn == "ca").map(|p| p.ids.len()).collect::<Vec<_>>(), vec![100, 50, 100, 50, 100, 50]);
        assert_eq!(s.per_account[0].posted, 450);
        // Marks moved to the newest posted message; next tick continues there.
        assert_eq!(s.marks.get("A", "Inbox"), Some(NOW - DAY + 450));
        // The 250 ms pauses went through the clock, not a real sleep.
        assert!(clock.slept.get() >= 4 * SCRIPT_PAUSE_MS);
    }

    #[test]
    fn a_401_stops_with_the_key_message() {
        // 401 on /messages stops the whole run: no more reads, no heartbeats.
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_n("a", NOW - DAY, 2, true)]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None), connected("b@x.com", "cb", None)])
            .on_messages(|_, _, _| Err(ApiError::KeyRejected));
        let s = run(input(vec![account("A", &["a@x.com"]), account("B", &["b@x.com"])]), &r, &api);
        assert_eq!(s.overall, Overall::KeyRejected);
        assert_eq!(read_scripts(&r).len(), 1);
        assert!(api.beats.borrow().is_empty());
        assert_eq!(serde_json::to_value(s.overall).unwrap(), "key-rejected");

        // 403 on /accounts: nothing is read at all.
        let r = FakeRunner::new(vec![ping_ok()]);
        let mut api = FakeApi::new(vec![]);
        api.on_accounts = Box::new(|_| Err(ApiError::KeyRejected));
        let s = run(input(vec![account("A", &["a@x.com"])]), &r, &api);
        assert_eq!(s.overall, Overall::KeyRejected);
        assert_eq!(r.scripts.borrow().len(), 1);
    }

    #[test]
    fn the_mark_only_advances_after_a_successful_post() {
        // Chunk 1 posts (mark moves), chunk 2's second batch fails with a
        // 500 (mark stays at chunk 1's newest).
        let r = FakeRunner::new(vec![
            ping_ok(),
            mailboxes(&["Inbox"]),
            read_n("a", NOW - DAY, 3, true),
            read_n("b", NOW - DAY + 3, 120, true),
        ]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None)]).on_messages(|_, _, m| {
            if m.iter().any(|x| x.internet_message_id == "b101@x") {
                Err(ApiError::Unavailable)
            } else {
                Ok(PostResult { accepted: m.len() as u32, ..Default::default() })
            }
        });
        let s = run(input(vec![account("A", &["a@x.com"])]), &r, &api);
        assert_eq!(s.marks.get("A", "Inbox"), Some(NOW - DAY + 3));
        assert_eq!(s.overall, Overall::FlowUnavailable);
        // The second chunk's read started from chunk 1's newest.
        assert_eq!(read_scripts(&r)[1]["since"], NOW - DAY + 3);
    }

    #[test]
    fn a_null_resume_from_resets_to_60_days() {
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_n("a", 0, 0, false)]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None)]);
        let mut i = input(vec![account("A", &["a@x.com"])]);
        i.marks.set("A", "Inbox", NOW - 1_000);
        i.marks.set("A", "Gone", NOW - 1_000);
        let s = run(i, &r, &api);
        assert_eq!(read_scripts(&r)[0]["since"], NOW - INITIAL_LOOKBACK_MS);
        assert_eq!(s.marks.get("A", "Inbox"), Some(NOW - INITIAL_LOOKBACK_MS));
        assert_eq!(s.marks.get("A", "Gone"), None, "FLOW's view replaces the account's marks");
    }

    #[test]
    fn a_deselected_account_is_never_read() {
        // FLOW still knows B (from an earlier selection) but only A is ticked.
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_n("a", NOW - DAY, 1, false)]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None), connected("b@x.com", "cb", None)]);
        let s = run(input(vec![account("A", &["a@x.com"])]), &r, &api);
        let regs = api.registered.borrow();
        assert_eq!(regs[0].iter().map(|a| a.apple_account_id.as_str()).collect::<Vec<_>>(), vec!["A"]);
        let scripts = r.scripts.borrow();
        assert!(scripts.iter().skip(1).all(|sc| params_of(sc)["account"] == "A"));
        assert!(api.posts.borrow().iter().all(|p| p.conn == "ca"));
        assert!(api.beats.borrow().iter().all(|b| b.0 == "ca"));
        assert_eq!(s.per_account.len(), 1);
    }

    // ── controller tests ──

    #[test]
    fn skips_when_mail_isnt_running() {
        let r = FakeRunner::ok(r#"{"notRunning":true}"#);
        let api = FakeApi::new(vec![]);
        let mut a = account("A", &["a@x.com"]);
        a.connection_id = Some("ca".into());
        let mut b = account("B", &["b@x.com"]);
        b.connection_id = Some("cb".into());
        let c = account("C", &["c@x.com"]); // never registered: no connection
        let s = run(input(vec![a, b, c]), &r, &api);
        assert_eq!(s.overall, Overall::MailNotRunning);
        assert_eq!(r.scripts.borrow().len(), 1, "ping only");
        assert!(read_scripts(&r).is_empty());
        assert!(api.registered.borrow().is_empty());
        assert_eq!(
            *api.beats.borrow(),
            vec![
                ("ca".into(), HeartbeatStatus::MailNotRunning, None),
                ("cb".into(), HeartbeatStatus::MailNotRunning, None),
            ]
        );
    }

    #[test]
    fn maps_1743_to_permission_denied() {
        let r = FakeRunner::new(vec![Err(jxa::classify_osascript_failure(
            "execution error: Not authorized to send Apple events to Mail. (-1743)",
        ))]);
        let api = FakeApi::new(vec![]);
        let mut a = account("A", &["a@x.com"]);
        a.connection_id = Some("ca".into());
        let s = run(input(vec![a]), &r, &api);
        assert_eq!(s.overall, Overall::PermissionDenied);
        assert!(read_scripts(&r).is_empty());
        assert_eq!(*api.beats.borrow(), vec![("ca".into(), HeartbeatStatus::PermissionDenied, None)]);
    }

    #[test]
    fn chunking_at_200_with_a_cap_of_2000() {
        // Each chunk returns 190 (budget hit) with more: ten chunks of 200
        // asked, then the 11th asks only for the 100 left under the cap.
        let mut outs = vec![ping_ok(), mailboxes(&["Inbox", "Archive"])];
        for c in 0..10 {
            outs.push(read_n(&format!("c{c}-"), NOW - DAY + c * 190, 190, true));
        }
        outs.push(read_n("c10-", NOW - DAY + 1_900, 100, true));
        let r = FakeRunner::new(outs);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None)]);
        let s = run(input(vec![account("A", &["a@x.com"])]), &r, &api);

        let reads = read_scripts(&r);
        assert_eq!(reads.len(), 11);
        let limits: Vec<_> = reads.iter().map(|p| p["limit"].as_u64().unwrap()).collect();
        assert_eq!(limits, [vec![200; 10], vec![100]].concat());
        assert!(reads.iter().all(|p| p["budgetMs"] == SCRIPT_BUDGET_MS && p["mailbox"] == "Inbox"));
        // since: mark − 10 min first, then each chunk's newest.
        assert_eq!(reads[0]["since"], NOW - INITIAL_LOOKBACK_MS);
        for c in 1..11 {
            assert_eq!(reads[c]["since"], NOW - DAY + c as i64 * 190);
        }
        assert_eq!(s.per_account[0].read, 2_000);
        assert_eq!(state_of(&s, "A"), Some(AccountState::CatchingUp));
        assert_eq!(s.marks.get("A", "Inbox"), Some(NOW - DAY + 2_000));
    }

    #[test]
    fn flow_resume_from_wins_over_a_newer_local_mark() {
        let r = FakeRunner::new(vec![
            ping_ok(),
            mailboxes(&["Inbox", "Sent Items"]),
            read_n("a", 0, 0, false),
            read_n("b", 0, 0, false),
        ]);
        let api = FakeApi::new(vec![connected("A@X.com", "ca", Some(&[("Inbox", "2025-10-20T10:00:00.000Z")]))]);
        let mut i = input(vec![account("A", &["a@x.com"])]);
        i.marks.set("A", "Inbox", NOW - 1_000); // newer than FLOW's
        i.marks.set("A", "Sent Items", NOW - 1_000);
        let s = run(i, &r, &api);
        let flow = iso_to_ms("2025-10-20T10:00:00.000Z").unwrap();
        let reads = read_scripts(&r);
        assert_eq!(reads[0]["since"], flow - OVERLAP_MS);
        // FLOW has no key for Sent Items: 60 days back, local mark ignored.
        assert_eq!(reads[1]["since"], NOW - INITIAL_LOOKBACK_MS);
        assert_eq!(s.marks.get("A", "Inbox"), Some(flow));
        assert_eq!(s.marks.get("A", "Sent Items"), Some(NOW - INITIAL_LOOKBACK_MS));
        // Matched despite FLOW lower-casing the address.
        assert_eq!(state_of(&s, "A"), Some(AccountState::Ok));
    }

    #[test]
    fn a_batch_is_split_by_the_8_mb_cap() {
        // 100 messages × 100 KB bodies ≈ 10 MB: under the count cap, over the
        // byte cap, so two posts.
        let msgs: Vec<_> = (1..=100).map(|i| msg(&format!("m{i}@x"), NOW - DAY + i, 100 * 1024)).collect();
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_msgs(msgs, false)]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None)]);
        let s = run(input(vec![account("A", &["a@x.com"])]), &r, &api);
        let posts = api.posts.borrow();
        assert_eq!(posts.len(), 2);
        assert!(posts.iter().all(|p| p.bytes <= BATCH_MAX_BYTES && p.mailbox == "Inbox"));
        assert_eq!(posts.iter().map(|p| p.ids.len()).sum::<usize>(), 100);
        assert_eq!(s.per_account[0].posted, 100);

        // Pure split: count cap and byte cap.
        let small: Vec<ConnectorMessage> = (0..250).map(|i| conn_msg(&format!("{i}"), 10)).collect();
        let b = split_batches("c", "Inbox", small, BATCH_MAX_BYTES);
        assert_eq!(b.iter().map(Vec::len).collect::<Vec<_>>(), vec![100, 100, 50]);
        let big: Vec<ConnectorMessage> = (0..5).map(|i| conn_msg(&format!("{i}"), 1_000)).collect();
        let limit = crate::mail_connector::flow_client::messages_body_len("c", "Inbox", &big[..2]);
        let b = split_batches("c", "Inbox", big, limit);
        assert_eq!(b.iter().map(Vec::len).collect::<Vec<_>>(), vec![2, 2, 1]);
        assert!(split_batches("c", "Inbox", vec![], BATCH_MAX_BYTES).is_empty());
    }

    fn conn_msg(id: &str, body: usize) -> ConnectorMessage {
        let raw: crate::mail_connector::RawMailMessage = serde_json::from_value(msg(id, NOW, body)).unwrap();
        crate::mail_connector::parse::to_connector_message(&raw).unwrap()
    }

    #[test]
    fn a_400_batch_is_reposted_one_at_a_time() {
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_n("a", NOW - DAY, 5, false)]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None)]).on_messages(|_, _, m| {
            if m.iter().any(|x| x.internet_message_id == "a2@x" || x.internet_message_id == "a4@x") {
                Err(ApiError::BadRequest)
            } else {
                Ok(PostResult { accepted: m.len() as u32, ..Default::default() })
            }
        });
        let s = run(input(vec![account("A", &["a@x.com"])]), &r, &api);
        let posts = api.posts.borrow();
        assert_eq!(posts[0].ids.len(), 5);
        assert_eq!(posts.len(), 6, "the batch, then each message alone");
        assert!(posts[1..].iter().all(|p| p.ids.len() == 1));
        assert_eq!(s.per_account[0].posted, 3);
        assert_eq!(s.per_account[0].skipped, 2);
        // A bad message can't wedge the mailbox: the mark moves past it.
        assert_eq!(s.marks.get("A", "Inbox"), Some(NOW - DAY + 5));
        assert_eq!(state_of(&s, "A"), Some(AccountState::Ok));

        // Only 400/413 count as "skip": FLOW going away during the re-post
        // still stops the run and keeps the mark.
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_n("a", NOW - DAY, 3, false)]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None)]).on_messages(|_, _, m| {
            if m.len() > 1 { Err(ApiError::BadRequest) } else { Err(ApiError::Unavailable) }
        });
        let s = run(input(vec![account("A", &["a@x.com"])]), &r, &api);
        assert_eq!(s.overall, Overall::FlowUnavailable);
        assert_eq!(api.posts.borrow().len(), 2, "stopped at the first failing single post");
        assert_eq!(s.per_account[0].skipped, 0);
        assert_eq!(s.marks.get("A", "Inbox"), Some(NOW - INITIAL_LOOKBACK_MS));
    }

    #[test]
    fn rate_limit_or_5xx_stops_the_run_and_keeps_the_marks() {
        for err in [ApiError::Unavailable] {
            let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_n("a", NOW - DAY, 2, true)]);
            let e = err.clone();
            let api = FakeApi::new(vec![connected("a@x.com", "ca", Some(&[("Inbox", "2025-10-25T10:00:00.000Z")])), connected("b@x.com", "cb", None)])
                .on_messages(move |_, _, _| Err(e.clone()));
            let s = run(input(vec![account("A", &["a@x.com"]), account("B", &["b@x.com"])]), &r, &api);
            assert_eq!(s.overall, Overall::FlowUnavailable);
            assert_eq!(s.marks.get("A", "Inbox"), iso_to_ms("2025-10-25T10:00:00.000Z"));
            assert_eq!(read_scripts(&r).len(), 1, "B not read");
            assert!(api.beats.borrow().is_empty());
            assert_eq!(state_of(&s, "B"), None, "unchanged");
        }
        // 429 and 5xx are Unavailable at the HTTP layer.
        assert_eq!(crate::mail_connector::flow_client::classify_status(429), ApiError::Unavailable);
        assert_eq!(crate::mail_connector::flow_client::classify_status(502), ApiError::Unavailable);

        // On /accounts too.
        let r = FakeRunner::new(vec![ping_ok()]);
        let mut api = FakeApi::new(vec![]);
        api.on_accounts = Box::new(|_| Err(ApiError::Unavailable));
        let s = run(input(vec![account("A", &["a@x.com"])]), &r, &api);
        assert_eq!(s.overall, Overall::FlowUnavailable);
        assert_eq!(r.scripts.borrow().len(), 1);
    }

    #[test]
    fn invalid_aliases_are_dropped_and_an_account_with_none_is_no_address() {
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_n("a", 0, 0, false)]);
        let api = FakeApi::new(vec![connected("ben@sparks.example", "ca", None)]);
        let mut a = account("A", &["not an address", "ben@sparks.example", "b@@x", "Ben@Sparks.example", "b2@sparks.example"]);
        a.account.name = "S".repeat(300);
        let s = run(input(vec![a, account("N", &["nope", ""])]), &r, &api);
        let regs = api.registered.borrow();
        assert_eq!(regs[0].len(), 1, "N isn't registered");
        assert_eq!(regs[0][0].address, "ben@sparks.example");
        assert_eq!(regs[0][0].addresses, vec!["ben@sparks.example", "b2@sparks.example"]);
        assert_eq!(regs[0][0].display_name.len(), MAX_DISPLAY_NAME);
        assert_eq!(state_of(&s, "N"), Some(AccountState::NoAddress));
        assert_eq!(state_of(&s, "A"), Some(AccountState::Ok));

        for ok in ["a@b.co", "first.last+tag@sub.example.org", "o'neil@example.ie"] {
            assert!(registration(&account("x", &[ok]).account).is_some(), "{ok}");
        }
        for bad in ["", "a", "@b.co", "a@", "a@b", "a b@c.co", "<a@b.co>", "a@b..co", ".a@b.co", "a@-b.co", "a\u{0}@b.co"] {
            assert!(registration(&account("x", &[bad]).account).is_none(), "{bad:?}");
        }
        let many: Vec<String> = (0..60).map(|i| format!("u{i}@x.com")).collect();
        let refs: Vec<&str> = many.iter().map(String::as_str).collect();
        assert_eq!(registration(&account("x", &refs).account).unwrap().addresses.len(), MAX_ADDRESSES);
    }

    #[test]
    fn productionpoint_accounts_are_never_registered_or_read() {
        // Coordinator update: FLOW already reads productionpoint.co.uk
        // mailboxes directly. Such an account (e.g. selected before this
        // rule) is skipped: not registered, not read, no heartbeat.
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_n("b", NOW - DAY, 1, false)]);
        let api = FakeApi::new(vec![connected("b@x.com", "cb", None)]);
        let mut p = account("P", &["ben@sparks.example", "ben@productionpoint.co.uk"]);
        p.connection_id = Some("cp".into());
        let s = run(input(vec![p.clone(), account("B", &["b@x.com"])]), &r, &api);
        assert_eq!(state_of(&s, "P"), Some(AccountState::Direct));
        assert_eq!(state_of(&s, "B"), Some(AccountState::Ok));
        assert_eq!(api.registered.borrow()[0].iter().map(|a| a.apple_account_id.as_str()).collect::<Vec<_>>(), vec!["B"]);
        assert!(r.scripts.borrow().iter().skip(1).all(|sc| params_of(sc)["account"] == "B"));
        assert!(api.beats.borrow().iter().all(|b| b.0 == "cb"));

        // Not even a mail-not-running heartbeat for its old connection.
        let r = FakeRunner::ok(r#"{"notRunning":true}"#);
        let api = FakeApi::new(vec![]);
        run(input(vec![p.clone()]), &r, &api);
        assert!(api.beats.borrow().is_empty());

        // The status shows "direct" without a run.
        let cfg = ConnectorConfig { flow_url: Some("https://flow.test".into()), selected: vec![p] };
        let st = build_status(&cfg, &RuntimeSnapshot::default(), true, NOW);
        assert_eq!(st.accounts[0].state, AccountState::Direct);
        assert_eq!(serde_json::to_value(AccountState::Direct).unwrap(), "direct");
    }

    #[test]
    fn a_refused_account_doesnt_fail_the_others() {
        // The coordinator's exact refused-item shape, plus other refusals.
        let resp: AccountsResponse = serde_json::from_value(serde_json::json!({ "accounts": [
            { "address": "a@x.com", "connectionId": "ca", "status": "connected", "resumeFrom": null },
            { "address": "b@x.com", "connectionId": null, "status": "refused",
              "error": "already-connected-directly", "resumeFrom": null },
            { "address": "c@x.com", "connectionId": null, "status": "refused", "error": "too-many-mailboxes", "resumeFrom": null },
            { "address": "d@x.com", "connectionId": null, "status": "connected", "resumeFrom": null },
            { "address": "e@x.com", "connectionId": "ce", "status": "something-new", "resumeFrom": null }
        ] }))
        .unwrap();
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_n("a", NOW - DAY, 1, false)]);
        let mut api = FakeApi::new(vec![]);
        api.on_accounts = Box::new(move |_| Ok(resp.clone()));
        let ids = ["A", "B", "C", "D", "E"];
        let sel = ids.iter().map(|i| account(i, &[&format!("{}@x.com", i.to_lowercase())])).collect();
        let s = run(input(sel), &r, &api);
        assert_eq!(state_of(&s, "A"), Some(AccountState::Ok));
        assert_eq!(state_of(&s, "B"), Some(AccountState::Direct));
        assert_eq!(state_of(&s, "C"), Some(AccountState::Error));
        assert_eq!(state_of(&s, "D"), Some(AccountState::Error), "connected without a connectionId");
        assert_eq!(state_of(&s, "E"), Some(AccountState::Error));
        assert_eq!(api.posted_ids(), vec!["a1@x"]);
        assert_eq!(*api.beats.borrow(), vec![("ca".into(), HeartbeatStatus::Ok, None)]);
        assert_eq!(s.per_account[1].connection_id, None);
        assert_eq!(s.overall, Overall::Error);

        // A productionpoint.co.uk address in a refusal is "direct" whatever
        // the error says.
        let pp = account("P", &["ben@productionpoint.co.uk"]);
        assert_eq!(refused_state(&pp.account, Some("too-many-mailboxes")), AccountState::Direct);
        assert_eq!(refused_state(&account("Q", &["q@x.com"]).account, None), AccountState::Error);
    }

    #[test]
    fn a_whole_409_is_retried_one_account_at_a_time() {
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_n("a", NOW - DAY, 1, false)]);
        let mut api = FakeApi::new(vec![]);
        api.on_accounts = Box::new(|regs| match regs {
            [one] if one.apple_account_id == "A" => Ok(AccountsResponse { accounts: vec![connected("a@x.com", "ca", None)] }),
            _ => Err(ApiError::Conflict),
        });
        let s = run(input(vec![account("A", &["a@x.com"]), account("B", &["b@x.com"])]), &r, &api);
        let regs = api.registered.borrow();
        assert_eq!(regs.iter().map(Vec::len).collect::<Vec<_>>(), vec![2, 1, 1]);
        assert_eq!(state_of(&s, "A"), Some(AccountState::Ok));
        assert_eq!(state_of(&s, "B"), Some(AccountState::Error));
        assert_eq!(api.posted_ids(), vec!["a1@x"]);
        assert_eq!(s.overall, Overall::Error);

        // A key rejected on a re-post still stops everything.
        let r = FakeRunner::new(vec![ping_ok()]);
        let mut api = FakeApi::new(vec![]);
        api.on_accounts = Box::new(|regs| if regs.len() > 1 { Err(ApiError::Conflict) } else { Err(ApiError::KeyRejected) });
        let s = run(input(vec![account("A", &["a@x.com"]), account("B", &["b@x.com"])]), &r, &api);
        assert_eq!(s.overall, Overall::KeyRejected);
        assert_eq!(api.registered.borrow().len(), 2);
    }

    // ── review round 1: compound cursor (I1) and catch-up progress (I2) ──

    /// A Rust model of the read script (filter on the compound cursor, sort by
    /// (date, id), limit, `more`), so cursor handling can be followed across
    /// chunks and ticks. `jxa::read_script_resumes_inside_a_same_second_group`
    /// checks the real JS against the same semantics.
    struct SimMail {
        inbox: RefCell<Vec<(i64, i64)>>, // (dateReceived, Mail id)
        reads: RefCell<Vec<serde_json::Value>>,
    }
    impl SimMail {
        fn new(msgs: Vec<(i64, i64)>) -> Self {
            SimMail { inbox: RefCell::new(msgs), reads: RefCell::new(Vec::new()) }
        }
    }
    impl ScriptRunner for SimMail {
        fn run_jxa(&self, script: &str) -> Result<String, ScriptError> {
            let p = params_of(script);
            if script.contains(jxa::READ_MAILBOX_BODY) {
                self.reads.borrow_mut().push(p.clone());
                let since = p["since"].as_i64().unwrap();
                let after = p["afterId"].as_i64();
                let limit = p["limit"].as_u64().unwrap() as usize;
                let mut due: Vec<(i64, i64)> = self
                    .inbox
                    .borrow()
                    .iter()
                    .copied()
                    .filter(|&(t, id)| t > since || (t == since && after.is_some_and(|a| id > a)))
                    .collect();
                due.sort();
                let out: Vec<_> = due
                    .iter()
                    .take(limit)
                    .map(|&(t, id)| {
                        let mut m = msg(&format!("m{id}@x"), t, 4);
                        m["mailId"] = id.into();
                        m
                    })
                    .collect();
                let more = out.len() < due.len();
                Ok(serde_json::json!({ "ok": { "messages": out, "more": more } }).to_string())
            } else if script.contains(jxa::LIST_MAILBOXES_BODY) {
                mailboxes(&["Inbox"])
            } else {
                ping_ok()
            }
        }
    }

    /// FLOW model: dedupes by Message-ID; `resumeFrom` is the newest
    /// receivedAt it stored per mailbox, `null` before it stored anything.
    #[derive(Default)]
    struct FlowState {
        seen: std::collections::HashSet<String>,
        high: BTreeMap<String, i64>,
    }

    fn flow_sim() -> (FakeApi, std::rc::Rc<RefCell<FlowState>>) {
        let st = std::rc::Rc::new(RefCell::new(FlowState::default()));
        let mut api = FakeApi::new(vec![]);
        let s1 = st.clone();
        api.on_accounts = Box::new(move |_| {
            let s = s1.borrow();
            let resume = (!s.high.is_empty())
                .then(|| s.high.iter().map(|(k, v)| (k.clone(), crate::mail_connector::parse::ms_to_iso(*v))).collect());
            Ok(AccountsResponse {
                accounts: vec![RegisteredAccount {
                    address: "a@x.com".into(),
                    connection_id: Some("ca".into()),
                    status: ConnectionStatus::Connected,
                    error: None,
                    resume_from: resume,
                }],
            })
        });
        let s2 = st.clone();
        api.on_messages = Box::new(move |_, mb, m| {
            let mut s = s2.borrow_mut();
            let (mut accepted, mut duplicates) = (0, 0);
            for x in m {
                if s.seen.insert(x.internet_message_id.clone()) {
                    accepted += 1;
                    let t = iso_to_ms(&x.received_at).unwrap();
                    let h = s.high.entry(mb.to_string()).or_insert(t);
                    *h = (*h).max(t);
                } else {
                    duplicates += 1;
                }
            }
            Ok(PostResult { accepted, duplicates, high_water: None })
        });
        (api, st)
    }

    /// One tick for account A at `now`, starting from `marks`.
    fn tick_at(sim: &SimMail, api: &FakeApi, marks: Marks, now: i64) -> RunSummary {
        let clock = FakeClock::new();
        clock.now.set(now);
        let mut i = input(vec![account("A", &["a@x.com"])]);
        i.marks = marks;
        run_once(i, sim, api, &clock)
    }

    fn reads_since(sim: &SimMail) -> Vec<(i64, Option<i64>)> {
        sim.reads.borrow().iter().map(|p| (p["since"].as_i64().unwrap(), p["afterId"].as_i64())).collect()
    }

    #[test]
    fn a_boundary_inside_a_same_second_group_loses_nothing() {
        // 450 messages share one second; 20 more arrive 20 min later. Chunks
        // of 200 cut the group twice; nothing is lost or read twice.
        let t = NOW - DAY;
        let mut msgs: Vec<(i64, i64)> = (1..=450).map(|id| (t, id)).collect();
        msgs.extend((451..=470).map(|id| (t + 20 * 60_000, id)));
        let sim = SimMail::new(msgs);
        let (api, flow) = flow_sim();
        let s = tick_at(&sim, &api, Marks::default(), NOW);

        assert_eq!(s.per_account[0].posted, 470);
        assert_eq!(s.per_account[0].duplicates, 0, "nothing re-read in the same run");
        assert_eq!(flow.borrow().seen.len(), 470);
        assert_eq!(
            reads_since(&sim),
            vec![(NOW - INITIAL_LOOKBACK_MS, None), (t, Some(200)), (t, Some(400))]
        );
        assert_eq!(
            s.marks.cursor("A", "Inbox"),
            Some(Cursor { ms: t + 20 * 60_000, after_id: Some(470), catching_up: false })
        );
    }

    #[test]
    fn a_dense_window_progresses_across_ticks() {
        // 2,500 messages in one minute: more than one tick's 2,000 cap.
        let t0 = NOW - DAY;
        let dense: Vec<(i64, i64)> = (1..=2_500).map(|id| (t0 + (id / 50) * 1_000, id)).collect();
        let sim = SimMail::new(dense);
        let (api, flow) = flow_sim();

        // Tick 1 (FLOW has nothing): 2,000 read, the cursor is catching up.
        let s1 = tick_at(&sim, &api, Marks::default(), NOW);
        assert_eq!(s1.per_account[0].read, 2_000);
        let c1 = s1.marks.cursor("A", "Inbox").unwrap();
        assert_eq!((c1.after_id, c1.catching_up), (Some(2_000), true));

        // Tick 2: resumes exactly at the cursor (no overlap): the last 500.
        sim.reads.borrow_mut().clear();
        let s2 = tick_at(&sim, &api, s1.marks, NOW + 300_000);
        assert_eq!(reads_since(&sim)[0], (c1.ms, Some(2_000)));
        assert_eq!((s2.per_account[0].read, s2.per_account[0].posted, s2.per_account[0].duplicates), (500, 500, 0));
        assert!(!s2.marks.cursor("A", "Inbox").unwrap().catching_up);
        assert_eq!(flow.borrow().seen.len(), 2_500);

        // Steady state: new mail arrives. Tick 3's 10-min overlap re-reads
        // the dense window and is cut by the cap again (all duplicates,
        // so FLOW's mark doesn't move and the cursor ends up behind it) …
        let late = t0 + 120_000;
        sim.inbox.borrow_mut().extend((2_501..=2_510).map(|id| (late, id)));
        sim.reads.borrow_mut().clear();
        let s3 = tick_at(&sim, &api, s2.marks, NOW + 600_000);
        let flow_mark = *flow.borrow().high.get("Inbox").unwrap();
        assert_eq!(reads_since(&sim)[0], (flow_mark - OVERLAP_MS, None));
        assert_eq!(s3.per_account[0].posted, 0);
        let c3 = s3.marks.cursor("A", "Inbox").unwrap();
        assert!(c3.catching_up && c3.ms < flow_mark);

        // … and tick 4 continues from that cursor instead of re-reading the
        // same window, so the new mail is reached.
        sim.reads.borrow_mut().clear();
        let s4 = tick_at(&sim, &api, s3.marks, NOW + 900_000);
        assert_eq!(reads_since(&sim)[0], (c3.ms, c3.after_id));
        assert_eq!(s4.per_account[0].posted, 10);
        assert!(!s4.marks.cursor("A", "Inbox").unwrap().catching_up);
    }

    /// One tick for A against a fixed FLOW `resumeFrom`; returns the first
    /// read's (since, afterId) and the summary.
    fn start_with(local: Option<Cursor>, resume: Option<&[(&str, &str)]>) -> ((i64, Option<i64>), RunSummary) {
        let sim = SimMail::new(vec![]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", resume)]);
        let mut marks = Marks::default();
        if let Some(c) = local {
            marks.set_cursor("A", "Inbox", c);
        }
        let s = tick_at(&sim, &api, marks, NOW);
        let first = reads_since(&sim)[0];
        (first, s)
    }

    #[test]
    fn a_flow_null_still_resets() {
        let local = Cursor { ms: NOW - 3_600_000, after_id: Some(5), catching_up: true };
        let (first, s) = start_with(Some(local), None);
        assert_eq!(first, (NOW - INITIAL_LOOKBACK_MS, None));
        assert_eq!(s.marks.cursor("A", "Inbox"), Some(Cursor::at(NOW - INITIAL_LOOKBACK_MS)));
    }

    #[test]
    fn flow_newer_than_the_local_cursor_wins() {
        let flow_iso = "2025-10-26T09:00:00.000Z"; // NOW − 1 h
        let flow = iso_to_ms(flow_iso).unwrap();
        let local = Cursor { ms: flow - 2 * 3_600_000, after_id: Some(5), catching_up: true };
        let (first, s) = start_with(Some(local), Some(&[("Inbox", flow_iso)]));
        assert_eq!(first, (flow - OVERLAP_MS, None));
        assert_eq!(s.marks.cursor("A", "Inbox").unwrap().ms, flow);

        // A catching-up cursor inside FLOW's overlap window is still resumed:
        // that's a catch-up through the overlap (see the dense-window test).
        let local = Cursor { ms: flow - 5 * 60_000, after_id: Some(9), catching_up: true };
        let (first, _) = start_with(Some(local), Some(&[("Inbox", flow_iso)]));
        assert_eq!(first, (flow - 5 * 60_000, Some(9)));
    }

    #[test]
    fn a_finished_mailbox_gets_the_10_min_overlap() {
        let flow_iso = "2025-10-26T09:00:00.000Z";
        let flow = iso_to_ms(flow_iso).unwrap();
        // Read to the end last time: FLOW's mark − 10 min, no afterId, even
        // though the local cursor is newer.
        let local = Cursor { ms: flow + 60_000, after_id: Some(7), catching_up: false };
        let (first, _) = start_with(Some(local), Some(&[("Inbox", flow_iso)]));
        assert_eq!(first, (flow - OVERLAP_MS, None));

        // Catching up and FLOW has no key for the mailbox: resume locally.
        let local = Cursor { ms: NOW - DAY, after_id: Some(3), catching_up: true };
        let (first, _) = start_with(Some(local), Some(&[("Archive", flow_iso)]));
        assert_eq!(first, (NOW - DAY, Some(3)));

        // An empty read to the end clears catching-up.
        let (_, s) = start_with(Some(local), Some(&[("Archive", flow_iso)]));
        assert!(!s.marks.cursor("A", "Inbox").unwrap().catching_up);
    }

    #[test]
    fn not_configured_makes_no_calls() {
        for i in [
            RunInput { has_key: false, ..input(vec![account("A", &["a@x.com"])]) },
            RunInput { flow_url: None, ..input(vec![account("A", &["a@x.com"])]) },
            input(vec![]),
        ] {
            let r = FakeRunner::new(vec![]);
            let api = FakeApi::new(vec![]);
            let s = run(i, &r, &api);
            assert_eq!(s.overall, Overall::NotConfigured);
            assert!(r.scripts.borrow().is_empty());
            assert_eq!(api.calls(), 0);
        }
    }

    #[test]
    fn no_read_script_is_built_for_an_excluded_mailbox() {
        // Task 2 review follow-up: runner level, the measured SPARKS list.
        let sparks = [
            "Conversation History", "Sync Issues", "Sync Issues/Conflicts", "Sync Issues/Local Failures",
            "Sync Issues/Server Failures", "Archive", "Journal", "Tasks", "Notes", "Inbox", "Outbox",
            "Drafts", "Sent Items", "Deleted Items", "Junk Email",
        ];
        let r = FakeRunner::new(vec![
            ping_ok(),
            mailboxes(&sparks),
            read_n("a", 0, 0, false),
            read_n("b", 0, 0, false),
            read_n("c", 0, 0, false),
        ]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None)]);
        run(input(vec![account("A", &["a@x.com"])]), &r, &api);
        let read: Vec<String> = read_scripts(&r).iter().map(|p| p["mailbox"].as_str().unwrap().to_string()).collect();
        assert_eq!(read, vec!["Archive", "Inbox", "Sent Items"]);
        assert!(read.iter().all(|m| !crate::mail_connector::is_excluded_mailbox(m)));
        assert_eq!(r.scripts.borrow().len(), 5, "nothing else was run");
    }

    #[test]
    fn script_errors_mark_the_account_and_heartbeat_the_class() {
        let r = FakeRunner::new(vec![
            ping_ok(),
            mailboxes(&["Inbox"]),
            Err(jxa::classify_osascript_failure("execution error: Can't get \"Secret\". (-1712)")),
            mailboxes(&["INBOX"]),
            read_n("b", NOW - DAY, 1, false),
        ]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None), connected("b@x.com", "cb", None)]);
        let s = run(input(vec![account("A", &["a@x.com"]), account("B", &["b@x.com"])]), &r, &api);
        assert_eq!(state_of(&s, "A"), Some(AccountState::Error));
        assert_eq!(state_of(&s, "B"), Some(AccountState::Ok));
        assert_eq!(
            *api.beats.borrow(),
            vec![
                ("ca".into(), HeartbeatStatus::Error, Some("osascript error -1712".into())),
                ("cb".into(), HeartbeatStatus::Ok, None),
            ]
        );
        assert_eq!(s.overall, Overall::Error);
    }

    #[test]
    fn mail_quitting_mid_read_keeps_the_partial_chunk() {
        let partial = serde_json::json!({ "ok": { "messages": [msg("p1@x", NOW - DAY + 1, 4)], "more": true, "notRunning": true } });
        let mut a = account("A", &["a@x.com"]);
        a.connection_id = Some("old".into());
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox", "Archive"]), Ok(partial.to_string())]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None)]);
        let s = run(input(vec![a]), &r, &api);
        assert_eq!(api.posted_ids(), vec!["p1@x"]);
        assert_eq!(s.marks.get("A", "Inbox"), Some(NOW - DAY + 1));
        assert_eq!(s.overall, Overall::MailNotRunning);
        assert_eq!(*api.beats.borrow(), vec![("ca".into(), HeartbeatStatus::MailNotRunning, None)]);
        assert_eq!(read_scripts(&r).len(), 1);
    }

    #[test]
    fn sent_today_resets_at_local_midnight() {
        let today = local_date(NOW);
        let mut a = account("A", &["a@x.com"]);
        a.sent_today = 7;
        a.sent_today_date = Some(today.clone());
        let mut b = account("B", &["b@x.com"]);
        b.sent_today = 9;
        b.sent_today_date = Some("2000-01-01".into());
        let r = FakeRunner::new(vec![
            ping_ok(),
            mailboxes(&["Inbox"]),
            read_n("a", NOW - DAY, 2, false),
            mailboxes(&["Inbox"]),
            read_n("b", NOW - DAY, 3, false),
        ]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None), connected("b@x.com", "cb", None)])
            .on_messages(|_, _, m| Ok(PostResult { accepted: m.len() as u32 - 1, duplicates: 1, high_water: None }));
        let s = run(input(vec![a, b]), &r, &api);
        assert_eq!((s.per_account[0].sent_today, s.per_account[0].duplicates), (8, 1));
        assert_eq!(s.per_account[1].sent_today, 2, "yesterday's 9 dropped");
        assert!(s.per_account.iter().all(|a| a.sent_today_date == today));
        assert_eq!(s.per_account[0].last_read_at, Some(NOW + 3 * SCRIPT_PAUSE_MS as i64 - SCRIPT_PAUSE_MS as i64));

        // The status view resets a stale count without a run.
        let mut cfg = ConnectorConfig { flow_url: Some("https://flow.test".into()), selected: input(vec![]).selected };
        let mut stale = account("B", &["b@x.com"]);
        stale.sent_today = 9;
        stale.sent_today_date = Some("2000-01-01".into());
        cfg.selected.push(stale);
        let st = build_status(&cfg, &RuntimeSnapshot::default(), true, NOW);
        assert_eq!(st.accounts[0].sent_today, 0);

        assert_ne!(local_date(NOW), local_date(NOW + DAY));
        assert_eq!(local_date(NOW).len(), 10);
    }

    #[test]
    fn connector_status_serialises_in_the_ui_shape() {
        let mut a = account("A", &["a@x.com"]);
        a.last_read_at = Some(NOW - 5);
        a.sent_today = 4;
        a.sent_today_date = Some(local_date(NOW));
        let cfg = ConnectorConfig {
            flow_url: Some("https://flow.test".into()),
            selected: vec![a, account("N", &["nope"]), account("P", &["p@x.com"])],
        };
        let mut rt = RuntimeSnapshot { running: true, last_run_at: Some(NOW - 10), overall: Some(Overall::Ok), ..Default::default() };
        rt.states.insert("A".into(), AccountState::CatchingUp);
        let v = serde_json::to_value(build_status(&cfg, &rt, true, NOW)).unwrap();
        assert_eq!(
            v,
            serde_json::json!({
                "running": true,
                "lastRunAt": NOW - 10,
                "overall": "ok",
                "accounts": [
                    { "id": "A", "name": "Account A", "address": "a@x.com", "state": "catching-up", "lastReadAt": NOW - 5, "sentToday": 4 },
                    { "id": "N", "name": "Account N", "address": "nope", "state": "no-address", "lastReadAt": null, "sentToday": 0 },
                    { "id": "P", "name": "Account P", "address": "p@x.com", "state": "pending", "lastReadAt": null, "sentToday": 0 }
                ]
            })
        );

        // Not configured wins over the last run; configured with no run is idle.
        assert_eq!(build_status(&cfg, &rt, false, NOW).overall, Overall::NotConfigured);
        let no_url = ConnectorConfig { flow_url: None, ..cfg.clone() };
        assert_eq!(build_status(&no_url, &rt, true, NOW).overall, Overall::NotConfigured);
        assert_eq!(build_status(&cfg, &RuntimeSnapshot::default(), true, NOW).overall, Overall::Idle);
        let none = ConnectorConfig { selected: vec![], ..cfg.clone() };
        assert_eq!(build_status(&none, &rt, true, NOW).overall, Overall::NotConfigured);

        for (o, s) in [
            (Overall::NotConfigured, "not-configured"), (Overall::Idle, "idle"), (Overall::MailNotRunning, "mail-not-running"),
            (Overall::PermissionDenied, "permission-denied"), (Overall::FlowUnavailable, "flow-unavailable"), (Overall::Error, "error"),
        ] {
            assert_eq!(serde_json::to_value(o).unwrap(), s);
        }
        for (a, s) in [(AccountState::Disconnected, "disconnected"), (AccountState::Error, "error"), (AccountState::Ok, "ok")] {
            assert_eq!(serde_json::to_value(a).unwrap(), s);
        }
    }

    #[test]
    fn the_summary_is_merged_into_the_current_selection() {
        let r = FakeRunner::new(vec![ping_ok(), mailboxes(&["Inbox"]), read_n("a", NOW - DAY, 2, false)]);
        let api = FakeApi::new(vec![connected("a@x.com", "ca", None)]);
        let s = run(input(vec![account("A", &["a@x.com"])]), &r, &api);

        // Mid-run the user also ticked C; A's bookkeeping lands, C is untouched.
        let mut cfg = ConnectorConfig { flow_url: None, selected: vec![account("A", &["a@x.com"]), account("C", &["c@x.com"])] };
        apply_summary(&mut cfg, &s);
        assert_eq!(cfg.selected[0].connection_id.as_deref(), Some("ca"));
        assert_eq!(cfg.selected[0].sent_today, 2);
        assert_eq!(cfg.selected[0].sent_today_date.as_deref(), Some(local_date(NOW).as_str()));
        assert!(cfg.selected[0].last_read_at.is_some());
        assert_eq!(cfg.selected[1], account("C", &["c@x.com"]));

        let rt = ConnectorRuntime::default();
        rt.record(&s);
        let snap = rt.snapshot();
        assert_eq!(snap.overall, Some(Overall::Ok));
        assert_eq!(snap.last_run_at, Some(s.finished_at));
        assert_eq!(snap.states.get("A"), Some(&AccountState::Ok));
    }

    #[test]
    fn run_now_wakes_are_coalesced() {
        let rt = ConnectorRuntime::default();
        assert!(!rt.wake(), "no loop yet");
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        rt.set_waker(tx);
        // While a run is in flight nobody is receiving: three clicks queue one.
        assert!(rt.wake());
        assert!(!rt.wake());
        assert!(!rt.wake());

        // drive: the queued wake runs immediately (no 15 s wait), then the
        // sender is dropped and the loop returns.
        let runs = Cell::new(0);
        drop(rt);
        drive(&rx, Duration::from_secs(3_600), Duration::from_secs(3_600), || runs.set(runs.get() + 1));
        assert_eq!(runs.get(), 1);

        // With no wakes, each timeout runs; dropping the sender ends the loop.
        let (tx, rx) = std::sync::mpsc::sync_channel::<()>(1);
        let tx = RefCell::new(Some(tx));
        let runs = Cell::new(0);
        drive(&rx, Duration::from_millis(1), Duration::from_millis(1), || {
            runs.set(runs.get() + 1);
            if runs.get() == 3 {
                tx.borrow_mut().take();
            }
        });
        assert_eq!(runs.get(), 3);
    }
}
