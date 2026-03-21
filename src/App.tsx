import { useState, useEffect, useCallback } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { invoke } from "@tauri-apps/api/core";
import { useSettings, useIsConfigured } from "./hooks/useSettings";
import { replayOfflineQueue } from "./hooks/useTasks";
import { useActiveTaskCount } from "./components/TaskList";
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

  const isPinned = settings.isPinned;
  const taskCount = useActiveTaskCount();

  const handleTogglePin = useCallback(() => {
    const newPinned = !isPinned;
    updateSettings({ isPinned: newPinned });
    getCurrentWindow().setAlwaysOnTop(newPinned).catch(() => {
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

  // Restore window position on mount
  useEffect(() => {
    const win = getCurrentWindow();
    const restore = async () => {
      try {
        if (settings.windowX !== null && settings.windowY !== null) {
          await win.setPosition(
            new (await import("@tauri-apps/api/dpi")).PhysicalPosition(
              settings.windowX,
              settings.windowY
            )
          );
        }
        await win.setSize(
          new (await import("@tauri-apps/api/dpi")).PhysicalSize(
            settings.windowWidth,
            settings.windowHeight
          )
        );
        if (settings.isPinned) {
          await win.setAlwaysOnTop(true);
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
        const overdue = tasks.filter((t) => !t.completed && t.dueDate && t.dueDate < todayStr);
        const dueToday = tasks.filter((t) => !t.completed && t.dueDate === todayStr);

        if (overdue.length > 0) {
          sendNotification({
            title: "Flow Tasks \u2014 Overdue",
            body: `You have ${overdue.length} overdue task${overdue.length > 1 ? "s" : ""}`,
          });
        }

        if (dueToday.length > 0) {
          const now = new Date();
          const [h, m] = settings.reminderTime.split(":").map(Number);
          if (now.getHours() === h && now.getMinutes() >= m && now.getMinutes() < m + 5) {
            sendNotification({
              title: "Flow Tasks \u2014 Due Today",
              body: `${dueToday.length} task${dueToday.length > 1 ? "s" : ""} due today`,
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
