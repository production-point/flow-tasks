import { describe, it, expect } from "vitest";
import {
  isMacPlatform,
  isDirectAccount,
  connectIds,
  formatAccountLine,
  formatRelativeTime,
  overallHelpText,
  dotColor,
  dotLabel,
  scriptErrorText,
  type ConnectorAccountStatus,
} from "../mail-connector";

const NOW = new Date(2026, 9, 26, 12, 0, 0).getTime();
const MIN = 60_000;
const HOUR = 60 * MIN;

function acct(over: Partial<ConnectorAccountStatus> = {}): ConnectorAccountStatus {
  return {
    id: "ABC",
    name: "SPARKS",
    address: "ben@sparks.example",
    state: "ok",
    lastReadAt: NOW - 2 * MIN,
    sentToday: 14,
    ...over,
  };
}

describe("isMacPlatform", () => {
  it("is true for Mac platform strings", () => {
    expect(isMacPlatform({ platform: "MacIntel" })).toBe(true);
    expect(isMacPlatform({ platform: "MacARM", userAgent: "" })).toBe(true);
  });

  it("falls back to the user agent when platform is empty", () => {
    expect(isMacPlatform({ platform: "", userAgent: "Mozilla/5.0 (Macintosh; Intel Mac OS X)" })).toBe(true);
  });

  it("is false for Windows and Linux", () => {
    expect(isMacPlatform({ platform: "Win32", userAgent: "Mozilla/5.0 (Windows NT 10.0)" })).toBe(false);
    expect(isMacPlatform({ platform: "Linux x86_64" })).toBe(false);
  });

  it("is false when there is no navigator", () => {
    expect(isMacPlatform(null)).toBe(false);
  });
});

describe("isDirectAccount", () => {
  it("matches any address ending @productionpoint.co.uk", () => {
    expect(isDirectAccount({ addresses: ["ben@productionpoint.co.uk"] })).toBe(true);
    expect(isDirectAccount({ addresses: ["x@gmail.com", "Ben@ProductionPoint.co.uk"] })).toBe(true);
  });

  it("is exact: subdomains and look-alikes are not direct", () => {
    expect(isDirectAccount({ addresses: ["a@mail.productionpoint.co.uk"] })).toBe(false);
    expect(isDirectAccount({ addresses: ["a@notproductionpoint.co.uk"] })).toBe(false);
    expect(isDirectAccount({ addresses: ["productionpoint.co.uk@gmail.com"] })).toBe(false);
  });

  it("is false with no addresses", () => {
    expect(isDirectAccount({ addresses: [] })).toBe(false);
  });
});

describe("connectIds", () => {
  const mail = [
    { id: "A", name: "SPARKS", addresses: ["ben@sparks.example"], kind: "unknown" },
    { id: "P", name: "PP", addresses: ["ben@productionpoint.co.uk"], kind: "unknown" },
    { id: "I", name: "iCloud", addresses: ["ben@icloud.com"], kind: "iCloud" },
  ];

  it("keeps ticked ids that Mail reports, in Mail's order, minus direct accounts", () => {
    expect(connectIds(mail, new Set(["I", "A", "P"]))).toEqual(["A", "I"]);
  });

  it("drops stale saved ids Mail no longer reports, so Connect can't send []", () => {
    // A saved selection whose account is gone from Mail gives nothing to
    // connect: the button must be disabled rather than clear the selection.
    expect(connectIds(mail, new Set(["GONE"]))).toEqual([]);
    expect(connectIds([], new Set(["A"]))).toEqual([]);
  });
});

describe("formatRelativeTime", () => {
  it("says never for null", () => {
    expect(formatRelativeTime(null, NOW)).toBe("never");
  });
  it("says just now under a minute (and for clock skew)", () => {
    expect(formatRelativeTime(NOW - 59_000, NOW)).toBe("just now");
    expect(formatRelativeTime(NOW + 5_000, NOW)).toBe("just now");
  });
  it("gives minutes then hours", () => {
    expect(formatRelativeTime(NOW - MIN, NOW)).toBe("1 min ago");
    expect(formatRelativeTime(NOW - 2 * MIN, NOW)).toBe("2 min ago");
    expect(formatRelativeTime(NOW - 59 * MIN, NOW)).toBe("59 min ago");
    expect(formatRelativeTime(NOW - HOUR, NOW)).toBe("1 h ago");
    expect(formatRelativeTime(NOW - 23 * HOUR, NOW)).toBe("23 h ago");
  });
  it("gives a date from 24 hours on", () => {
    const text = formatRelativeTime(NOW - 3 * 24 * HOUR, NOW);
    expect(text).not.toMatch(/ago|never|just now/);
    expect(text).toMatch(/23/);
  });
});

describe("formatAccountLine", () => {
  it("formats the ok line exactly", () => {
    expect(formatAccountLine(acct(), NOW)).toBe("SPARKS · last read 2 min ago · 14 sent today");
  });

  it("says never when the account hasn't been read", () => {
    expect(formatAccountLine(acct({ state: "pending", lastReadAt: null, sentToday: 0 }), NOW)).toBe(
      "SPARKS · last read never · 0 sent today",
    );
  });

  it("appends catching up", () => {
    expect(formatAccountLine(acct({ state: "catching-up" }), NOW)).toBe(
      "SPARKS · last read 2 min ago · 14 sent today · catching up",
    );
  });

  it("appends the state text for disconnected and error", () => {
    expect(formatAccountLine(acct({ state: "disconnected" }), NOW)).toBe(
      "SPARKS · last read 2 min ago · 14 sent today · Disconnected in FLOW",
    );
    expect(formatAccountLine(acct({ state: "error" }), NOW)).toBe(
      "SPARKS · last read 2 min ago · 14 sent today · Error",
    );
  });

  it("direct and no-address accounts are never read, so they show only the state", () => {
    expect(formatAccountLine(acct({ state: "direct", lastReadAt: null, sentToday: 0 }), NOW)).toBe(
      "SPARKS · Already connected directly to FLOW",
    );
    expect(formatAccountLine(acct({ state: "no-address", lastReadAt: null, sentToday: 0 }), NOW)).toBe(
      "SPARKS · No email address",
    );
  });
});

describe("overallHelpText", () => {
  it("has the verbatim help text for each problem", () => {
    expect(overallHelpText("mail-not-running")).toBe("Open Apple Mail (it can stay in the background).");
    expect(overallHelpText("permission-denied")).toBe(
      "Allow Flow Tasks to control Mail in System Settings › Privacy & Security › Automation.",
    );
    expect(overallHelpText("key-rejected")).toBe(
      "Key rejected: create a new Mail connector key in FLOW › Settings › API Keys.",
    );
    expect(overallHelpText("flow-unavailable")).toBe("FLOW isn't reachable right now; it will retry.");
    expect(overallHelpText("error")).toBe("The last read hit an error; it will retry.");
  });

  it("is null when there is nothing to say", () => {
    expect(overallHelpText("ok")).toBeNull();
    expect(overallHelpText("idle")).toBeNull();
    expect(overallHelpText("not-configured")).toBeNull();
    expect(overallHelpText(undefined)).toBeNull();
  });
});

describe("dotColor / dotLabel", () => {
  it("is green for ok and idle", () => {
    expect(dotColor("ok")).toBe("#22c55e");
    expect(dotColor("idle")).toBe("#22c55e");
  });
  it("is amber for mail-not-running and flow-unavailable", () => {
    expect(dotColor("mail-not-running")).toBe("#f59e0b");
    expect(dotColor("flow-unavailable")).toBe("#f59e0b");
  });
  it("is red for permission-denied, key-rejected and error", () => {
    expect(dotColor("permission-denied")).toBe("#ef4444");
    expect(dotColor("key-rejected")).toBe("#ef4444");
    expect(dotColor("error")).toBe("#ef4444");
  });
  it("is hidden for not-configured and unknown status", () => {
    expect(dotColor("not-configured")).toBeNull();
    expect(dotColor(undefined)).toBeNull();
  });
  it("has a tooltip for every visible dot", () => {
    for (const o of ["ok", "idle", "mail-not-running", "flow-unavailable", "permission-denied", "key-rejected", "error"] as const) {
      expect(dotLabel(o)).toMatch(/^Mail connector/);
    }
    expect(dotLabel("not-configured")).toBeNull();
  });
});

describe("scriptErrorText", () => {
  it("maps mail-not-running", () => {
    expect(scriptErrorText("mail-not-running")).toBe("Open Apple Mail (it can stay in the background).");
  });
  it("maps permission-denied", () => {
    expect(scriptErrorText("permission-denied")).toBe(
      "Allow Flow Tasks to control Mail in System Settings › Privacy & Security › Automation.",
    );
  });
  it("shows the error class for anything else, without the script-error prefix", () => {
    expect(scriptErrorText("script-error: 1728")).toBe("Couldn't read Apple Mail accounts. (1728)");
    expect(scriptErrorText(new Error("script-error: timeout"))).toBe("Couldn't read Apple Mail accounts. (timeout)");
  });
  it("copes with empty and non-string errors", () => {
    expect(scriptErrorText("")).toBe("Couldn't read Apple Mail accounts.");
    expect(scriptErrorText(undefined)).toBe("Couldn't read Apple Mail accounts.");
    expect(scriptErrorText({ weird: true })).toBe("Couldn't read Apple Mail accounts.");
  });
  it("caps a long error class", () => {
    const text = scriptErrorText("x".repeat(500));
    expect(text.length).toBeLessThan(120);
  });
});
