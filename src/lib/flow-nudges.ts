import { fetch as tauriFetch } from "@tauri-apps/plugin-http";

/**
 * FLOW's Mac reminders ("nudges"): `GET /api/v1/assistant/nudges` with the
 * tasks key. While FLOW answers `enabled: true`, FLOW's reminders replace
 * Flow Tasks' own overdue / due-today pings (Ben's "Replace" ruling, spec §9).
 * `enabled: false` (another user, the assistant off, Mac reminders off) or a
 * definite refusal means the local reminders carry on unchanged.
 */

export interface NudgeItem {
  key: string;
}

export interface NudgesAnswer {
  enabled: boolean;
  items: NudgeItem[];
  title?: string;
  body?: string;
  url?: string;
  /** Only if a future FLOW sends its quiet hours ("HH:MM"). */
  quietStart?: string;
  quietEnd?: string;
}

/** The last answer FLOW gave, and when. */
export interface LastNudgesAnswer {
  enabled: boolean;
  at: number;
  quietStart?: string;
  quietEnd?: string;
}

/** Keys shown today, so a restart never shows the same reminder twice. */
export interface NudgeGuard {
  day: string;
  keys: string[];
}

export type ReminderMode = "flow" | "local";

/** How long FLOW stays in charge after its last `enabled: true` while it
 *  can't be reached (network, 5xx). Past this, the local reminders return. */
export const FLOW_HOLD_MS = 30 * 60 * 1000;

/** Polled every 5 minutes, and on launch. */
export const NUDGE_POLL_MS = 5 * 60 * 1000;

/** The notification plugin has no click actions on desktop (its desktop
 *  `show()` is fire-and-forget), so the body says where to look instead. */
export const OPEN_FLOW_LINE = "Open FLOW to see it";

/** A failed nudges request: `status` is the HTTP status, 0 for a network
 *  failure. Never carries a response body. */
export class NudgesError extends Error {
  constructor(public status: number) {
    super(status ? `nudges ${status}` : "nudges unreachable");
  }
}

/** FLOW can't be reached right now (network, timeout, 429, 5xx): keep the
 *  previous mode for a while. Anything else (401/403/404 …) is a definite
 *  "not for this key" and means local reminders at once. */
export function isTransient(e: unknown): boolean {
  if (!(e instanceof NudgesError)) return true;
  return e.status === 0 || e.status === 429 || e.status >= 500;
}

async function httpFetch(url: string, init: RequestInit): Promise<Response> {
  const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
  try {
    return await tauriFetch(url, init);
  } catch (err) {
    if (inTauri || typeof globalThis.fetch !== "function") throw err;
    return globalThis.fetch(url, init); // dev browser only
  }
}

function base(apiUrl: string): string {
  return apiUrl.replace(/\/+$/, "");
}

async function request(url: string, init: RequestInit): Promise<Response> {
  let res: Response;
  try {
    res = await httpFetch(url, init);
  } catch {
    throw new NudgesError(0);
  }
  if (!res.ok) throw new NudgesError(res.status);
  return res;
}

const HHMM = /^\d{2}:\d{2}$/;

/** `GET /api/v1/assistant/nudges`. Anything but a strict `enabled: true`
 *  reads as disabled. */
export async function fetchNudges(apiUrl: string, apiKey: string): Promise<NudgesAnswer> {
  const res = await request(`${base(apiUrl)}/api/v1/assistant/nudges`, {
    method: "GET",
    headers: { Authorization: `Bearer ${apiKey}` },
  });
  let raw: unknown;
  try {
    raw = await res.json();
  } catch {
    throw new NudgesError(res.status || 0);
  }
  const o = (raw && typeof raw === "object" ? raw : {}) as Record<string, unknown>;
  const str = (v: unknown) => (typeof v === "string" ? v : undefined);
  const items = Array.isArray(o.items)
    ? o.items
        .map((i) => (i && typeof i === "object" ? str((i as Record<string, unknown>).key) : undefined))
        .filter((k): k is string => !!k)
        .map((key) => ({ key }))
    : [];
  const quietStart = str(o.quietStart);
  const quietEnd = str(o.quietEnd);
  return {
    enabled: o.enabled === true,
    items,
    title: str(o.title),
    body: str(o.body),
    url: str(o.url),
    ...(quietStart && HHMM.test(quietStart) ? { quietStart } : {}),
    ...(quietEnd && HHMM.test(quietEnd) ? { quietEnd } : {}),
  };
}

/** `POST /api/v1/assistant/nudges/delivered` after showing a reminder.
 *  Idempotent on FLOW's side (per key per London day). */
export async function postDelivered(apiUrl: string, apiKey: string, keys: string[]): Promise<void> {
  if (keys.length === 0) return;
  await request(`${base(apiUrl)}/api/v1/assistant/nudges/delivered`, {
    method: "POST",
    headers: { Authorization: `Bearer ${apiKey}`, "Content-Type": "application/json" },
    body: JSON.stringify({ keys: keys.slice(0, 50) }),
  });
}

/** FLOW is in charge while its last answer was `enabled: true` and is no
 *  older than `FLOW_HOLD_MS`. Every successful poll refreshes `at`, so the
 *  hold only runs out while FLOW can't be reached. */
export function decideMode(last: LastNudgesAnswer | null, now: number): ReminderMode {
  if (!last || !last.enabled) return "local";
  return now - last.at <= FLOW_HOLD_MS ? "flow" : "local";
}

/** The new "last answer" after a poll: an answer replaces it, a definite
 *  refusal switches to local now, a transient failure keeps it (so
 *  `decideMode` holds the previous mode for up to 30 minutes). */
export function recordPoll(
  last: LastNudgesAnswer | null,
  outcome: { answer: NudgesAnswer } | { error: unknown },
  now: number,
): LastNudgesAnswer | null {
  if ("answer" in outcome) {
    const { enabled, quietStart, quietEnd } = outcome.answer;
    return { enabled, at: now, ...(quietStart ? { quietStart } : {}), ...(quietEnd ? { quietEnd } : {}) };
  }
  return isTransient(outcome.error) ? last : { enabled: false, at: now };
}

/** Show only if at least one key hasn't been shown today on this Mac. */
export function shouldShow(keys: string[], guard: NudgeGuard, today: string): boolean {
  if (keys.length === 0) return false;
  if (guard.day !== today) return true;
  return keys.some((k) => !guard.keys.includes(k));
}

/** The guard after showing `keys`. */
export function rememberShown(keys: string[], guard: NudgeGuard, today: string): NudgeGuard {
  const prior = guard.day === today ? guard.keys : [];
  return { day: today, keys: [...new Set([...prior, ...keys])].slice(-200) };
}

/** The notification to show for FLOW's answer: its title and body exactly,
 *  plus where to look (there's no click action on desktop). */
export function notificationFor(answer: NudgesAnswer): { title: string; body: string } {
  const body = answer.body ? `${answer.body}\n${OPEN_FLOW_LINE}` : OPEN_FLOW_LINE;
  return { title: answer.title ?? "FLOW", body };
}

/** Settings' line while FLOW's reminders are in charge. */
export function flowRemindersText(last: LastNudgesAnswer | null): string {
  const quiet = last?.quietStart && last?.quietEnd ? `${last.quietEnd}–${last.quietStart}` : "07:30–19:00";
  return `Reminders come from FLOW (quiet hours ${quiet})`;
}

/** Local `YYYY-MM-DD` for the guard. */
export function localDay(now: number): string {
  const d = new Date(now);
  const p = (n: number) => String(n).padStart(2, "0");
  return `${d.getFullYear()}-${p(d.getMonth() + 1)}-${p(d.getDate())}`;
}

/** What one reminder tick needs, injected so the switch logic is tested
 *  without timers, Tauri or the network. */
export interface ReminderTickDeps {
  now: number;
  apiUrl: string;
  apiKey: string | null;
  last: LastNudgesAnswer | null;
  guard: NudgeGuard;
  fetchNudges: (apiUrl: string, apiKey: string) => Promise<NudgesAnswer>;
  postDelivered: (apiUrl: string, apiKey: string, keys: string[]) => Promise<void>;
  /** Show one notification. */
  notify: (n: { title: string; body: string }) => void;
  /** The existing local overdue / due-today check, unchanged. */
  runLocal: () => void;
  saveLast: (last: LastNudgesAnswer | null) => void;
  saveGuard: (guard: NudgeGuard) => void;
}

/**
 * One tick, every 5 minutes: ask FLOW, decide the mode, then show exactly one
 * kind of reminder. `flow` shows FLOW's bundle (if it has new keys) and posts
 * `delivered`; `local` runs the existing local check. Never both.
 */
export async function reminderTick(d: ReminderTickDeps): Promise<ReminderMode> {
  if (!d.apiKey || !d.apiUrl) {
    d.saveLast(null);
    d.runLocal();
    return "local";
  }
  let answer: NudgesAnswer | null = null;
  let last: LastNudgesAnswer | null;
  try {
    answer = await d.fetchNudges(d.apiUrl, d.apiKey);
    last = recordPoll(d.last, { answer }, d.now);
  } catch (error) {
    last = recordPoll(d.last, { error }, d.now);
  }
  d.saveLast(last);
  const mode = decideMode(last, d.now);
  if (mode === "local") {
    d.runLocal();
    return mode;
  }
  // FLOW is in charge. With no answer this tick (FLOW unreachable inside the
  // hold), nothing is shown at all.
  if (!answer || !answer.enabled || answer.items.length === 0) return mode;
  const keys = answer.items.map((i) => i.key);
  const today = localDay(d.now);
  if (shouldShow(keys, d.guard, today)) {
    d.notify(notificationFor(answer));
    d.saveGuard(rememberShown(keys, d.guard, today));
  }
  // FLOW handing back keys this Mac already showed means its `delivered`
  // didn't land: post again (idempotent), without showing them again.
  try {
    await d.postDelivered(d.apiUrl, d.apiKey, keys);
  } catch {
    // Next tick: FLOW hands the same keys back and this posts again.
  }
  return mode;
}
