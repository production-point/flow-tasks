import { check, type Update } from "@tauri-apps/plugin-updater";

/**
 * Check GitHub releases (via the signed `latest.json` manifest) for a newer
 * version. Returns the `Update` handle when one is available, `null` otherwise.
 * Safe to call outside Tauri — in a plain browser the plugin import throws and
 * we swallow the error so dev mode stays usable.
 */
export async function checkForUpdate(): Promise<Update | null> {
  try {
    return await check();
  } catch {
    return null;
  }
}

/**
 * Drive `Update.download()` and report progress in the range [0, 1].
 * When content-length is unknown we emit indeterminate ticks (-1) so callers
 * can still show motion.
 */
export async function downloadUpdate(
  update: Update,
  onProgress: (progress: number) => void,
): Promise<void> {
  let downloaded = 0;
  let total = 0;
  await update.download((event) => {
    switch (event.event) {
      case "Started":
        total = event.data.contentLength ?? 0;
        onProgress(total > 0 ? 0 : -1);
        break;
      case "Progress":
        downloaded += event.data.chunkLength;
        onProgress(total > 0 ? Math.min(1, downloaded / total) : -1);
        break;
      case "Finished":
        onProgress(1);
        break;
    }
  });
}

/**
 * Run the installer. On Windows this spawns the NSIS/WiX installer which
 * relaunches the app — our process exits as part of the handoff.
 */
export async function installUpdate(update: Update): Promise<void> {
  await update.install();
}
