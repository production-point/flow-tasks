import { describe, it, expect, vi, beforeEach } from "vitest";

const httpFetch = vi.fn();
vi.mock("@tauri-apps/plugin-http", () => ({ fetch: (...a: unknown[]) => httpFetch(...a) }));

import {
  FLOW_HOLD_MS,
  NudgesError,
  OPEN_FLOW_LINE,
  decideMode,
  fetchNudges,
  flowRemindersText,
  notificationFor,
  postDelivered,
  recordPoll,
  rememberShown,
  shouldShow,
  localDay,
  reminderTick,
  type LastNudgesAnswer,
  type NudgeGuard,
} from "../flow-nudges";

const NOW = new Date(2026, 9, 27, 10, 0, 0).getTime();
const MIN = 60_000;

function json(status: number, body: unknown) {
  return new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
}

beforeEach(() => httpFetch.mockReset());

describe("decideMode", () => {
  it("is local with no answer yet, or when FLOW says disabled", () => {
    expect(decideMode(null, NOW)).toBe("local");
    expect(decideMode({ enabled: false, at: NOW }, NOW)).toBe("local");
  });

  it("is flow while the last answer was enabled", () => {
    expect(decideMode({ enabled: true, at: NOW }, NOW)).toBe("flow");
    expect(decideMode({ enabled: true, at: NOW - 5 * MIN }, NOW)).toBe("flow");
  });

  it("holds flow for up to 30 minutes of errors, then falls back to local", () => {
    // Review Focus 4: FLOW unreachable or 5xx keeps the previous answer.
    let last = recordPoll(null, { answer: { enabled: true, items: [] } }, NOW);
    for (const err of [new NudgesError(0), new NudgesError(503), new NudgesError(429), new TypeError("boom")]) {
      last = recordPoll(last, { error: err }, NOW + 20 * MIN);
      expect(last).toEqual({ enabled: true, at: NOW });
      expect(decideMode(last, NOW + 20 * MIN)).toBe("flow");
    }
    expect(decideMode(last, NOW + FLOW_HOLD_MS)).toBe("flow");
    expect(decideMode(last, NOW + FLOW_HOLD_MS + 1)).toBe("local");
    // A local previous mode stays local through errors.
    const off = recordPoll(null, { answer: { enabled: false, items: [] } }, NOW);
    expect(decideMode(recordPoll(off, { error: new NudgesError(500) }, NOW + MIN), NOW + MIN)).toBe("local");
  });

  it("switches to local at once on a definite refusal (401, 403, 404)", () => {
    const last = recordPoll(null, { answer: { enabled: true, items: [] } }, NOW);
    for (const status of [401, 403, 404]) {
      const next = recordPoll(last, { error: new NudgesError(status) }, NOW + MIN);
      expect(decideMode(next, NOW + MIN)).toBe("local");
    }
  });
});

describe("shouldShow and the local guard", () => {
  it("shows new keys once per day on this Mac", () => {
    // Review Focus 5: a restart on the same Mac doesn't repeat it.
    const empty = { day: "", keys: [] };
    expect(shouldShow([], empty, "2026-10-27")).toBe(false);
    expect(shouldShow(["task:1"], empty, "2026-10-27")).toBe(true);
    const g = rememberShown(["task:1", "suggestion:2"], empty, "2026-10-27");
    expect(shouldShow(["task:1"], g, "2026-10-27")).toBe(false);
    expect(shouldShow(["suggestion:2", "task:1"], g, "2026-10-27")).toBe(false);
    expect(shouldShow(["task:1", "task:3"], g, "2026-10-27")).toBe(true);
    // A new day starts clean.
    expect(shouldShow(["task:1"], g, "2026-10-28")).toBe(true);
    expect(rememberShown(["task:9"], g, "2026-10-28")).toEqual({ day: "2026-10-28", keys: ["task:9"] });
    expect(rememberShown(["task:1", "task:3"], g, "2026-10-27").keys).toEqual(["task:1", "suggestion:2", "task:3"]);
  });

  it("formats the local day", () => {
    expect(localDay(NOW)).toBe("2026-10-27");
  });
});

describe("fetchNudges / postDelivered", () => {
  it("sends the tasks key and parses FLOW's bundle", async () => {
    httpFetch.mockResolvedValueOnce(
      json(200, {
        enabled: true,
        items: [{ key: "task:1", kind: "overdue", line: "Roundhouse RAMS" }, { key: "suggestion:2" }, { nope: 1 }],
        title: "2 overdue",
        body: "Roundhouse RAMS\nCall Sparks",
        url: "https://flow.example/assistant/today",
      }),
    );
    const a = await fetchNudges("https://flow.example/", "ftsk_tasks");
    const [url, init] = httpFetch.mock.calls[0];
    expect(url).toBe("https://flow.example/api/v1/assistant/nudges");
    expect(init.method).toBe("GET");
    expect(init.headers.Authorization).toBe("Bearer ftsk_tasks");
    expect(a).toEqual({
      enabled: true,
      items: [{ key: "task:1" }, { key: "suggestion:2" }],
      title: "2 overdue",
      body: "Roundhouse RAMS\nCall Sparks",
      url: "https://flow.example/assistant/today",
    });
  });

  it("reads enabled: false (another user, Mac reminders off) as local", async () => {
    httpFetch.mockResolvedValueOnce(json(200, { enabled: false, items: [] }));
    const a = await fetchNudges("https://flow.example", "k");
    expect(a.enabled).toBe(false);
    expect(decideMode(recordPoll({ enabled: true, at: NOW }, { answer: a }, NOW + MIN), NOW + MIN)).toBe("local");
    // Anything but a strict true is disabled.
    httpFetch.mockResolvedValueOnce(json(200, { enabled: "yes" }));
    expect((await fetchNudges("https://flow.example", "k")).enabled).toBe(false);
  });

  it("throws a status-only error, never the body", async () => {
    httpFetch.mockResolvedValueOnce(new Response("Subject: Acme invoice", { status: 503 }));
    const e = await fetchNudges("https://flow.example", "k").catch((x) => x);
    expect(e).toBeInstanceOf(NudgesError);
    expect(e.status).toBe(503);
    expect(String(e.message)).not.toContain("Acme");

    httpFetch.mockRejectedValueOnce(new Error("dns: flow.example"));
    const n = await fetchNudges("https://flow.example", "k").catch((x) => x);
    expect(n.status).toBe(0);
  });

  it("posts the delivered keys", async () => {
    httpFetch.mockResolvedValueOnce(json(200, { enabled: true, recorded: 2 }));
    await postDelivered("https://flow.example", "k", ["task:1", "suggestion:2"]);
    const [url, init] = httpFetch.mock.calls[0];
    expect(url).toBe("https://flow.example/api/v1/assistant/nudges/delivered");
    expect(init.method).toBe("POST");
    expect(init.headers.Authorization).toBe("Bearer k");
    expect(JSON.parse(init.body)).toEqual({ keys: ["task:1", "suggestion:2"] });

    await postDelivered("https://flow.example", "k", []);
    expect(httpFetch).toHaveBeenCalledTimes(1);
  });
});

describe("what is shown", () => {
  it("uses FLOW's title and body exactly, plus where to look", () => {
    expect(notificationFor({ enabled: true, items: [{ key: "task:1" }], title: "Chased: Roundhouse RAMS", body: "Sam replied" })).toEqual({
      title: "Chased: Roundhouse RAMS",
      body: `Sam replied\n${OPEN_FLOW_LINE}`,
    });
  });

  it("shows FLOW's quiet hours in Settings, else the default", () => {
    expect(flowRemindersText(null)).toBe("Reminders come from FLOW (quiet hours 07:30–19:00)");
    expect(flowRemindersText({ enabled: true, at: NOW, quietStart: "20:00", quietEnd: "08:00" })).toBe(
      "Reminders come from FLOW (quiet hours 08:00–20:00)",
    );
  });
});

describe("reminderTick", () => {
  function deps(over: Partial<Parameters<typeof reminderTick>[0]> = {}) {
    const calls = { notify: [] as Array<{ title: string; body: string }>, local: 0, delivered: [] as string[][] };
    const state = { last: null as LastNudgesAnswer | null, guard: { day: "", keys: [] as string[] } };
    const d = {
      now: NOW,
      apiUrl: "https://flow.example",
      apiKey: "k",
      last: null,
      guard: { day: "", keys: [] },
      fetchNudges: vi.fn(async () => ({ enabled: true, items: [{ key: "task:1" }], title: "1 overdue", body: "RAMS" })),
      postDelivered: vi.fn(async (_u: string, _k: string, keys: string[]) => {
        calls.delivered.push(keys);
      }),
      notify: (n: { title: string; body: string }) => calls.notify.push(n),
      runLocal: () => {
        calls.local += 1;
      },
      saveLast: (l: LastNudgesAnswer | null) => {
        state.last = l;
      },
      saveGuard: (g: NudgeGuard) => {
        state.guard = g;
      },
      ...over,
    };
    return { d, calls, state };
  }

  it("in FLOW mode shows FLOW's one notification, posts delivered and skips local pings", async () => {
    const { d, calls, state } = deps();
    expect(await reminderTick(d)).toBe("flow");
    expect(calls.notify).toEqual([{ title: "1 overdue", body: `RAMS\n${OPEN_FLOW_LINE}` }]);
    expect(calls.delivered).toEqual([["task:1"]]);
    expect(calls.local).toBe(0);
    expect(state.guard).toEqual({ day: "2026-10-27", keys: ["task:1"] });
    expect(state.last).toEqual({ enabled: true, at: NOW });
  });

  it("with enabled: false or no key runs the local reminders only", async () => {
    let t = deps({ fetchNudges: vi.fn(async () => ({ enabled: false, items: [] })) });
    expect(await reminderTick(t.d)).toBe("local");
    expect(t.calls.local).toBe(1);
    expect(t.calls.notify).toEqual([]);
    expect(t.calls.delivered).toEqual([]);

    t = deps({ apiKey: null });
    expect(await reminderTick(t.d)).toBe("local");
    expect(t.calls.local).toBe(1);
    expect(t.d.fetchNudges).not.toHaveBeenCalled();
  });

  it("never shows both kinds in one tick while FLOW is unreachable", async () => {
    // Review Focus 4: inside the hold nothing local fires; after it, only local.
    const failing = vi.fn(async () => {
      throw new NudgesError(502);
    });
    let t = deps({ fetchNudges: failing, last: { enabled: true, at: NOW - 10 * MIN } });
    expect(await reminderTick(t.d)).toBe("flow");
    expect(t.calls.local).toBe(0);
    expect(t.calls.notify).toEqual([]);
    expect(t.state.last).toEqual({ enabled: true, at: NOW - 10 * MIN });

    t = deps({ fetchNudges: failing, last: { enabled: true, at: NOW - 31 * MIN } });
    expect(await reminderTick(t.d)).toBe("local");
    expect(t.calls.local).toBe(1);
    expect(t.calls.notify).toEqual([]);
  });

  it("doesn't show again after a restart, but re-posts delivered", async () => {
    // Review Focus 5: FLOW hands the key back (its delivered didn't land).
    const t = deps({ guard: { day: "2026-10-27", keys: ["task:1"] } });
    expect(await reminderTick(t.d)).toBe("flow");
    expect(t.calls.notify).toEqual([]);
    expect(t.calls.delivered).toEqual([["task:1"]]);
  });

  it("shows nothing for an empty bundle (quiet hours, cap, nothing due)", async () => {
    const t = deps({ fetchNudges: vi.fn(async () => ({ enabled: true, items: [] })) });
    expect(await reminderTick(t.d)).toBe("flow");
    expect(t.calls.notify).toEqual([]);
    expect(t.calls.local).toBe(0);
    expect(t.calls.delivered).toEqual([]);
  });

  it("a failed delivered post doesn't undo the guard", async () => {
    const t = deps({
      postDelivered: vi.fn(async () => {
        throw new NudgesError(503);
      }),
    });
    expect(await reminderTick(t.d)).toBe("flow");
    expect(t.calls.notify.length).toBe(1);
    expect(t.state.guard.keys).toEqual(["task:1"]);
  });
});
