import { create } from "zustand";
import { persist } from "zustand/middleware";
import type { AppSettings } from "../lib/types";

// Display string only — the actual shortcut is registered natively in the Rust
// shell (Cmd+Option+T on macOS, Ctrl+Alt+T elsewhere). Keep them in sync.
const DEFAULT_HOTKEY =
  typeof navigator !== "undefined" && /Mac/i.test(navigator.platform)
    ? "Cmd+Option+T"
    : "Ctrl+Alt+T";

const DEFAULT_SETTINGS: AppSettings = {
  apiUrl: "",
  pollInterval: 30,
  hotkey: DEFAULT_HOTKEY,
  notificationsEnabled: true,
  reminderTime: "09:00",
  startOnLogin: false,
  windowX: null,
  windowY: null,
  windowWidth: 350,
  windowHeight: 500,
  activeTab: "all",
  isPinned: false,
};

interface SettingsState {
  settings: AppSettings;
  apiKey: string | null;
  updateSettings: (partial: Partial<AppSettings>) => void;
  setApiKey: (key: string | null) => void;
}

export const useSettings = create<SettingsState>()(
  persist(
    (set) => ({
      settings: DEFAULT_SETTINGS,
      apiKey: null,
      updateSettings: (partial) =>
        set((state) => ({
          settings: {
            ...state.settings,
            ...partial,
            pollInterval: Math.max(10, partial.pollInterval ?? state.settings.pollInterval),
          },
        })),
      setApiKey: (key) => set({ apiKey: key }),
    }),
    { name: "flow-tasks-settings" }
  )
);

export function useIsConfigured(): boolean {
  return useSettings((s) => Boolean(s.settings.apiUrl && s.apiKey));
}
