import { useState, useEffect, useCallback, useRef } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { getCurrentWindow, availableMonitors } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import type { Update } from "@tauri-apps/plugin-updater";
import { useSettings, useIsConfigured } from "./hooks/useSettings";
import { replayOfflineQueue } from "./hooks/useTasks";
import { useActiveTaskCount } from "./components/TaskList";
import { toDateString } from "./lib/date-utils";
import { checkForUpdate, downloadUpdate, installUpdate } from "./lib/update-checker";
import { isWindowOnScreen } from "./lib/window-bounds";
import TitleBar from "./components/TitleBar";
import SearchBar from "./components/SearchBar";
import TaskList from "./components/TaskList";
import TaskEditor from "./components/TaskEditor";
import Settings from "./components/Settings";
import type { Task } from "./lib/types";

const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 30_000,
      retry: 1,
    },
  },
});

function AppShell() {
  const settings = useSettings((s) => s.settings);
  const updateSettings = useSettings((s) => s.updateSettings);
  const configured = useIsConfigured();

  const [showSettings, setShowSettings] = useState(false);
  const [showSearch, setShowSearch] = useState(false);
  const [searchQuery, setSearchQuery] = useState("");
  const [editingTask, setEditingTask] = useState<Task | null>(null);
  const [updateState, setUpdateState] = useState<
    | { status: "idle" }
    | { status: "downloading"; version: string; progress: number }
    | { status: "ready"; version: string }
    | { status: "error"; message: string }
  >({ status: "idle" });
  const updateRef = useRef<Update | null>(null);

  const isPinned = settings.isPinned;
  const taskCount = useActiveTaskCount();

  const handleTogglePin = useCallback(() => {
    const newPinned = !isPinned;
    updateSettings({ isPinned: newPinned });
    // Route through the Rust command (not setAlwaysOnTop directly) so the shell
    // records the pin flag the macOS hide-on-blur handler reads — a pinned,
    // torn-off window must stay put instead of vanishing on focus loss.
    invoke("toggle_pin", { pinned: newPinned }).catch(() => {
      // Tauri API not available in dev browser
    });
  }, [isPinned, updateSettings]);

  const handleToggleSettings = useCallback(() => {
    setShowSettings((prev) => !prev);
    setEditingTask(null);
  }, []);

  const handleToggleSearch = useCallback(() => {
    setShowSearch((prev) => {
      if (prev) setSearchQuery("");
      return !prev;
    });
  }, []);

  const handleCloseSearch = useCallback(() => {
    setShowSearch(false);
    setSearchQuery("");
  }, []);

  const handleEditTask = useCallback((task: Task) => {
    setEditingTask(task);
    setShowSettings(false);
  }, []);

  const handleCloseEditor = useCallback(() => {
    setEditingTask(null);
  }, []);

  const handleCloseSettings = useCallback(() => {
    setShowSettings(false);
  }, []);

  // Restore window position on mount.
  // Saved coords can be stale (e.g., user disconnected the monitor they were
  // on) — if they no longer land on any visible screen, discard them and let
  // Tauri center. Also clamp size to the configured minimums so a corrupt
  // persisted value can't spawn a 0×0 window.
  useEffect(() => {
    const win = getCurrentWindow();
    const restore = async () => {
      try {
        const { PhysicalPosition, PhysicalSize } = await import("@tauri-apps/api/dpi");

        const width = Math.max(300, settings.windowWidth);
        const height = Math.max(400, settings.windowHeight);
        await win.setSize(new PhysicalSize(width, height));

        if (settings.windowX !== null && settings.windowY !== null) {
          const monitors = await availableMonitors();
          const bounds = monitors.map((m) => ({
            x: m.position.x,
            y: m.position.y,
            width: m.size.width,
            height: m.size.height,
          }));
          if (isWindowOnScreen(settings.windowX, settings.windowY, width, height, bounds)) {
            await win.setPosition(new PhysicalPosition(settings.windowX, settings.windowY));
          } else {
            // Off-screen coordinates from a previous monitor setup — recenter
            // and clear so we don't fight the user next launch.
            await win.center();
            updateSettings({ windowX: null, windowY: null });
          }
        } else {
          await win.center();
        }

        if (settings.isPinned) {
          // Sync the shell's pin flag on startup so a restored floating window
          // isn't auto-hidden by the macOS blur handler.
          await invoke("toggle_pin", { pinned: true });
        }
      } catch {
        // Tauri API not available in dev browser
      }
    };
    restore();
  }, []); // eslint-disable-line react-hooks/exhaustive-deps

  // Save window position on move/resize
  useEffect(() => {
    let cleanup: (() => void) | undefined;

    const setup = async () => {
      try {
        const win = getCurrentWindow();
        const unlistenMove = await win.onMoved(({ payload }) => {
          updateSettings({ windowX: payload.x, windowY: payload.y });
        });
        const unlistenResize = await win.onResized(({ payload }) => {
          updateSettings({ windowWidth: payload.width, windowHeight: payload.height });
        });
        cleanup = () => {
          unlistenMove();
          unlistenResize();
        };
      } catch {
        // Tauri API not available in dev browser
      }
    };
    setup();

    return () => cleanup?.();
  }, [updateSettings]);

  // Replay offline queue on window focus
  useEffect(() => {
    const handleFocus = () => {
      replayOfflineQueue((msg) => console.warn("[offline-queue]", msg));
    };
    window.addEventListener("focus", handleFocus);
    return () => window.removeEventListener("focus", handleFocus);
  }, []);

  // Load API key from OS keychain on startup
  useEffect(() => {
    invoke<string | null>("get_api_key")
      .then((key) => {
        if (key) useSettings.getState().setApiKey(key);
      })
      .catch(() => {}); // Ignore errors (not in Tauri context)
  }, []);

  // Notification effect — overdue and morning reminders
  useEffect(() => {
    if (!configured || !settings.notificationsEnabled) return;

    async function checkNotifications() {
      try {
        const { isPermissionGranted, requestPermission, sendNotification } = await import(
          "@tauri-apps/plugin-notification"
        );

        let granted = await isPermissionGranted();
        if (!granted) {
          const permission = await requestPermission();
          granted = permission === "granted";
        }
        if (!granted) return;

        const tasks = queryClient.getQueryData<Task[]>(["tasks"]);
        if (!tasks) return;

        const todayStr = new Date().toISOString().split("T")[0];
        const overdue = tasks.filter((t) => {
          if (t.completed || !t.dueDate) return false;
          const d = toDateString(t.dueDate);
          return d !== null && d < todayStr;
        });
        const dueToday = tasks.filter((t) => {
          if (t.completed || !t.dueDate) return false;
          return toDateString(t.dueDate) === todayStr;
        });

        if (overdue.length > 0) {
          const titles = overdue.slice(0, 3).map((t) => `- ${t.title}`);
          if (overdue.length > 3) titles.push(`... and ${overdue.length - 3} more`);
          sendNotification({
            title: "Flow Tasks \u2014 Overdue",
            body: titles.join("\n"),
          });
        }

        if (dueToday.length > 0) {
          const now = new Date();
          const [h, m] = settings.reminderTime.split(":").map(Number);
          if (now.getHours() === h && now.getMinutes() >= m && now.getMinutes() < m + 5) {
            const titles = dueToday.slice(0, 3).map((t) => `- ${t.title}`);
            if (dueToday.length > 3) titles.push(`... and ${dueToday.length - 3} more`);
            sendNotification({
              title: "Flow Tasks \u2014 Due Today",
              body: titles.join("\n"),
            });
          }
        }
      } catch {
        // Notification plugin not available (browser dev mode)
      }
    }

    const interval = setInterval(checkNotifications, 5 * 60 * 1000);
    checkNotifications();
    return () => clearInterval(interval);
  }, [configured, settings.notificationsEnabled, settings.reminderTime]);

  // Check for updates on startup, then silently background-download if one is
  // available. When download finishes we flip to "ready" and expose an
  // "Install & restart" button — the install step spawns the NSIS installer,
  // which exits our process and relaunches the new version.
  useEffect(() => {
    let cancelled = false;
    (async () => {
      const update = await checkForUpdate();
      if (cancelled || !update) return;
      updateRef.current = update;
      setUpdateState({ status: "downloading", version: update.version, progress: 0 });
      try {
        await downloadUpdate(update, (progress) => {
          if (cancelled) return;
          setUpdateState({ status: "downloading", version: update.version, progress });
        });
        if (cancelled) return;
        setUpdateState({ status: "ready", version: update.version });
      } catch (e) {
        if (cancelled) return;
        setUpdateState({ status: "error", message: e instanceof Error ? e.message : "Download failed" });
      }
    })();
    return () => { cancelled = true; };
  }, []);

  const handleInstallUpdate = useCallback(async () => {
    const update = updateRef.current;
    if (!update) return;
    try {
      await installUpdate(update);
    } catch (e) {
      setUpdateState({ status: "error", message: e instanceof Error ? e.message : "Install failed" });
    }
  }, []);

  const handleDismissUpdate = useCallback(() => {
    setUpdateState({ status: "idle" });
  }, []);

  // Determine content to show
  let content: React.ReactNode;
  if (editingTask) {
    content = <TaskEditor task={editingTask} onClose={handleCloseEditor} />;
  } else if (showSettings) {
    content = <Settings onClose={handleCloseSettings} />;
  } else if (!configured) {
    content = <Welcome onOpenSettings={() => setShowSettings(true)} />;
  } else {
    content = <TaskList searchQuery={searchQuery} onEditTask={handleEditTask} />;
  }

  return (
    <div
      className="flex flex-col h-screen"
      style={{ backgroundColor: "var(--flow-bg-primary)" }}
    >
      <TitleBar
        taskCount={taskCount}
        isPinned={isPinned}
        onTogglePin={handleTogglePin}
        onToggleSettings={handleToggleSettings}
        onToggleSearch={handleToggleSearch}
      />
      {updateState.status !== "idle" && (
        <div
          className="flex items-center justify-between px-3 py-1.5 text-xs"
          style={{ backgroundColor: "var(--flow-accent)", color: "#ffffff" }}
        >
          {updateState.status === "downloading" && (
            <>
              <span>
                Downloading v{updateState.version}
                {updateState.progress >= 0 ? ` — ${Math.round(updateState.progress * 100)}%` : "…"}
              </span>
              <button
                onClick={handleDismissUpdate}
                className="opacity-70 hover:opacity-100"
                aria-label="Dismiss"
              >
                &#x2715;
              </button>
            </>
          )}
          {updateState.status === "ready" && (
            <>
              <span>Update ready: v{updateState.version}</span>
              <div className="flex gap-2">
                <button
                  onClick={handleInstallUpdate}
                  className="underline font-medium"
                >
                  Install &amp; restart
                </button>
                <button
                  onClick={handleDismissUpdate}
                  className="opacity-70 hover:opacity-100"
                  aria-label="Dismiss"
                >
                  &#x2715;
                </button>
              </div>
            </>
          )}
          {updateState.status === "error" && (
            <>
              <span>Update failed: {updateState.message}</span>
              <button
                onClick={handleDismissUpdate}
                className="opacity-70 hover:opacity-100"
                aria-label="Dismiss"
              >
                &#x2715;
              </button>
            </>
          )}
        </div>
      )}
      {showSearch && (
        <SearchBar
          value={searchQuery}
          onChange={setSearchQuery}
          onClose={handleCloseSearch}
        />
      )}
      <main className="flex-1 overflow-hidden">{content}</main>
    </div>
  );
}

function Welcome({ onOpenSettings }: { onOpenSettings: () => void }) {
  return (
    <div className="flex flex-col items-center justify-center h-full gap-4 px-6 text-center">
      <h2 className="text-lg font-semibold" style={{ color: "var(--flow-text-primary)" }}>
        Welcome to Flow Tasks
      </h2>
      <p className="text-sm" style={{ color: "var(--flow-text-secondary)" }}>
        Connect to your FLOW instance to get started. You will need your API URL and an API key.
      </p>
      <button
        onClick={onOpenSettings}
        className="text-sm font-medium px-4 py-2 rounded transition-colors"
        style={{ backgroundColor: "var(--flow-accent)", color: "#ffffff" }}
      >
        Open Settings
      </button>
    </div>
  );
}

export default function App() {
  return (
    <QueryClientProvider client={queryClient}>
      <AppShell />
    </QueryClientProvider>
  );
}
