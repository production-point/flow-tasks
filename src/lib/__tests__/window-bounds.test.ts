import { describe, it, expect } from "vitest";
import { isWindowOnScreen, MIN_VISIBLE_PX } from "../window-bounds";

const primary = { x: 0, y: 0, width: 1920, height: 1080 };
const secondaryRight = { x: 1920, y: 0, width: 2560, height: 1440 };
const secondaryLeft = { x: -1920, y: 0, width: 1920, height: 1080 };

describe("isWindowOnScreen", () => {
  it("returns true for a window fully on the primary monitor", () => {
    expect(isWindowOnScreen(100, 100, 350, 500, [primary])).toBe(true);
  });

  it("returns true for a window fully on a secondary monitor", () => {
    expect(isWindowOnScreen(2000, 200, 350, 500, [primary, secondaryRight])).toBe(true);
  });

  it("returns true for a window straddling two monitors", () => {
    // Sits across the seam at x=1920, mostly on the right monitor.
    expect(isWindowOnScreen(1800, 100, 350, 500, [primary, secondaryRight])).toBe(true);
  });

  it("returns false when saved coordinates point to a disconnected monitor", () => {
    // External monitor that used to be at (3000, 0) is now unplugged.
    expect(isWindowOnScreen(3200, 100, 350, 500, [primary])).toBe(false);
  });

  it("returns false for a window entirely above the primary monitor", () => {
    expect(isWindowOnScreen(100, -2000, 350, 500, [primary])).toBe(false);
  });

  it("returns false when a negative-x monitor is disconnected", () => {
    // User had a left-side monitor that's now gone.
    expect(isWindowOnScreen(-1500, 100, 350, 500, [primary])).toBe(false);
  });

  it("returns true when a negative-x monitor is still attached", () => {
    expect(isWindowOnScreen(-1500, 100, 350, 500, [primary, secondaryLeft])).toBe(true);
  });

  it("rejects a sliver smaller than MIN_VISIBLE_PX", () => {
    // Only 10px of the window overlap the primary monitor on the x-axis.
    const x = primary.width - 10;
    expect(isWindowOnScreen(x, 100, 350, 500, [primary])).toBe(false);
  });

  it("accepts exactly MIN_VISIBLE_PX of overlap", () => {
    const x = primary.width - MIN_VISIBLE_PX;
    expect(isWindowOnScreen(x, 100, 350, 500, [primary])).toBe(true);
  });

  it("returns false when there are no monitors", () => {
    expect(isWindowOnScreen(0, 0, 350, 500, [])).toBe(false);
  });
});
