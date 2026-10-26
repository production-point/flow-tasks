// Pure helpers for the Apple Mail connector UI (Settings section + title-bar
// dot). Nothing here touches Tauri, the DOM, or message content.

export type ConnectorOverall =
  | "not-configured"
  | "idle"
  | "ok"
  | "mail-not-running"
  | "permission-denied"
  | "key-rejected"
  | "flow-unavailable"
  | "error";

export type ConnectorAccountState =
  | "pending"
  | "ok"
  | "catching-up"
  | "disconnected"
  | "no-address"
  | "direct"
  | "error";

export interface ConnectorAccountStatus {
  id: string;
  name: string;
  address: string;
  state: ConnectorAccountState;
  lastReadAt: number | null;
  sentToday: number;
}

export interface ConnectorStatus {
  running: boolean;
  lastRunAt: number | null;
  overall: ConnectorOverall;
  accounts: ConnectorAccountStatus[];
}

/** An Apple Mail account as `mail_connector_accounts` returns it. */
export interface MailAccount {
  id: string;
  name: string;
  addresses: string[];
  kind: string;
}

export interface ConnectorConfigView {
  flowUrl: string | null;
  selected: MailAccount[];
  hasKey: boolean;
}

export const DEFAULT_FLOW_URL = "https://flow.productionpoint.co.uk";
const DIRECT_DOMAIN = "@productionpoint.co.uk";

const MAIL_NOT_RUNNING_TEXT = "Open Apple Mail (it can stay in the background).";
const PERMISSION_DENIED_TEXT =
  "Allow Flow Tasks to control Mail in System Settings › Privacy & Security › Automation.";

/** Same test as useSettings.ts, with the navigator injectable for tests. */
export function isMacPlatform(
  nav: { platform?: string; userAgent?: string } | null = typeof navigator !== "undefined"
    ? navigator
    : null,
): boolean {
  if (!nav) return false; // not in a browser context
  return /Mac/i.test(nav.platform ?? "") || (!nav.platform && /Mac/i.test(nav.userAgent ?? ""));
}

/** Accounts on FLOW's own domain are connected to FLOW directly. Exact match,
 *  as in the Rust side: subdomains and look-alikes don't count. */
export function isDirectAccount(account: { addresses: string[] }): boolean {
  return account.addresses.some((a) => a.trim().toLowerCase().endsWith(DIRECT_DOMAIN));
}

/** The ids Connect sends to `mail_connector_set_selected`: ticked accounts
 *  that Mail just reported (the backend resolves ids against that read), in
 *  Mail's order, minus direct accounts. Connect is enabled only when this is
 *  non-empty, so stale saved ids can't enable it and send an empty list. */
export function connectIds(accounts: MailAccount[], ticked: ReadonlySet<string>): string[] {
  return accounts.filter((a) => ticked.has(a.id) && !isDirectAccount(a)).map((a) => a.id);
}

export function formatRelativeTime(at: number | null, now: number): string {
  if (at === null) return "never";
  const diff = now - at;
  if (diff < 60_000) return "just now";
  const mins = Math.floor(diff / 60_000);
  if (mins < 60) return `${mins} min ago`;
  const hours = Math.floor(diff / 3_600_000);
  if (hours < 24) return `${hours} h ago`;
  return new Date(at).toLocaleDateString("en-GB", { day: "numeric", month: "short" });
}

const STATE_SUFFIX: Partial<Record<ConnectorAccountState, string>> = {
  "catching-up": "catching up",
  disconnected: "Disconnected in FLOW",
  error: "Error",
};

/** "SPARKS · last read 2 min ago · 14 sent today", plus a state suffix.
 *  Direct and no-address accounts are never read, so they show only their state. */
export function formatAccountLine(account: ConnectorAccountStatus, now: number): string {
  if (account.state === "direct") return `${account.name} · Already connected directly to FLOW`;
  if (account.state === "no-address") return `${account.name} · No email address`;
  const parts = [
    account.name,
    `last read ${formatRelativeTime(account.lastReadAt, now)}`,
    `${account.sentToday} sent today`,
  ];
  const suffix = STATE_SUFFIX[account.state];
  if (suffix) parts.push(suffix);
  return parts.join(" · ");
}

export function overallHelpText(overall: ConnectorOverall | undefined): string | null {
  switch (overall) {
    case "mail-not-running":
      return MAIL_NOT_RUNNING_TEXT;
    case "permission-denied":
      return PERMISSION_DENIED_TEXT;
    case "key-rejected":
      return "Key rejected: create a new Mail connector key in FLOW › Settings › API Keys.";
    case "flow-unavailable":
      return "FLOW isn't reachable right now; it will retry.";
    case "error":
      return "The last read hit an error; it will retry.";
    default:
      return null;
  }
}

export const DOT_GREEN = "#22c55e";
export const DOT_AMBER = "#f59e0b";
export const DOT_RED = "#ef4444";

/** Title-bar dot colour; null means the dot is hidden. */
export function dotColor(overall: ConnectorOverall | undefined): string | null {
  switch (overall) {
    case "ok":
    case "idle":
      return DOT_GREEN;
    case "mail-not-running":
    case "flow-unavailable":
      return DOT_AMBER;
    case "permission-denied":
    case "key-rejected":
    case "error":
      return DOT_RED;
    default:
      return null;
  }
}

/** Tooltip for the dot; null when the dot is hidden. */
export function dotLabel(overall: ConnectorOverall | undefined): string | null {
  switch (overall) {
    case "ok":
      return "Mail connector: reading Apple Mail";
    case "idle":
      return "Mail connector: ready";
    case "mail-not-running":
      return "Mail connector: Apple Mail isn't open";
    case "flow-unavailable":
      return "Mail connector: FLOW isn't reachable";
    case "permission-denied":
      return "Mail connector: Mail access not allowed";
    case "key-rejected":
      return "Mail connector: key rejected by FLOW";
    case "error":
      return "Mail connector: last read hit an error";
    default:
      return null;
  }
}

/** Turn a rejected `mail_connector_accounts` call into user-facing text. */
export function scriptErrorText(err: unknown): string {
  const raw = typeof err === "string" ? err : err instanceof Error ? err.message : "";
  const text = raw.trim();
  if (text === "mail-not-running") return MAIL_NOT_RUNNING_TEXT;
  if (text === "permission-denied") return PERMISSION_DENIED_TEXT;
  const errorClass = text.replace(/^script-error:\s*/, "").slice(0, 60);
  return errorClass
    ? `Couldn't read Apple Mail accounts. (${errorClass})`
    : "Couldn't read Apple Mail accounts.";
}
