/**
 * Window-bounds validation helpers.
 *
 * Used when restoring persisted window coordinates on startup. If the user
 * disconnects an external monitor or changes resolution, the saved position
 * can land the window entirely off every visible screen, making the app
 * unreachable. See {@link isWindowOnScreen} for the check we apply.
 */

export interface MonitorBounds {
  x: number;
  y: number;
  width: number;
  height: number;
}

/**
 * Minimum visible area (in physical pixels, per axis) required for a window
 * to count as "on screen". A 1px sliver peeking onto a monitor should NOT
 * count — the user needs enough of the window to grab and drag.
 */
export const MIN_VISIBLE_PX = 50;

/**
 * Returns true if the given window rectangle overlaps any monitor by at least
 * {@link MIN_VISIBLE_PX} on both axes. Coordinates are in the same physical
 * pixel space Tauri's `PhysicalPosition` uses.
 */
export function isWindowOnScreen(
  winX: number,
  winY: number,
  winWidth: number,
  winHeight: number,
  monitors: MonitorBounds[],
): boolean {
  if (monitors.length === 0) return false;
  return monitors.some((m) => {
    const overlapX =
      Math.min(winX + winWidth, m.x + m.width) - Math.max(winX, m.x);
    const overlapY =
      Math.min(winY + winHeight, m.y + m.height) - Math.max(winY, m.y);
    return overlapX >= MIN_VISIBLE_PX && overlapY >= MIN_VISIBLE_PX;
  });
}
