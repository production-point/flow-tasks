import { useState } from "react";
import { useSettings } from "../hooks/useSettings";

interface SettingsProps {
  onClose: () => void;
}

export default function Settings({ onClose }: SettingsProps) {
  const settings = useSettings((s) => s.settings);
  const apiKey = useSettings((s) => s.apiKey);
  const updateSettings = useSettings((s) => s.updateSettings);
  const setApiKey = useSettings((s) => s.setApiKey);

  const [apiUrl, setApiUrl] = useState(settings.apiUrl);
  const [key, setKey] = useState(apiKey ?? "");
  const [pollInterval, setPollInterval] = useState(settings.pollInterval);
  const [notifications, setNotifications] = useState(settings.notificationsEnabled);
  const [startOnLogin, setStartOnLogin] = useState(settings.startOnLogin);

  const handleSave = () => {
    updateSettings({
      apiUrl,
      pollInterval: Math.max(10, pollInterval),
      notificationsEnabled: notifications,
      startOnLogin,
    });
    setApiKey(key || null);
    onClose();
  };

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div
        className="flex items-center justify-between px-3 py-2"
        style={{ borderBottom: "1px solid var(--flow-border)" }}
      >
        <span className="text-sm font-semibold" style={{ color: "var(--flow-text-primary)" }}>
          Settings
        </span>
        <button
          onClick={onClose}
          className="text-xs"
          style={{ color: "var(--flow-text-muted)" }}
        >
          &#x2715;
        </button>
      </div>

      {/* Fields */}
      <div className="flex-1 overflow-y-auto px-3 py-3 flex flex-col gap-4">
        <Field label="API URL" hint="Your FLOW instance base URL">
          <input
            type="text"
            value={apiUrl}
            onChange={(e) => setApiUrl(e.target.value)}
            placeholder="https://your-flow-instance.com"
            className="w-full text-sm rounded px-2 py-1.5 border-none outline-none"
            style={{
              backgroundColor: "var(--flow-bg-tertiary)",
              color: "var(--flow-text-primary)",
            }}
          />
        </Field>

        <Field label="API Key" hint="Generate from FLOW Settings > API Keys">
          <input
            type="password"
            value={key}
            onChange={(e) => setKey(e.target.value)}
            placeholder="ftsk_..."
            className="w-full text-sm rounded px-2 py-1.5 border-none outline-none"
            style={{
              backgroundColor: "var(--flow-bg-tertiary)",
              color: "var(--flow-text-primary)",
            }}
          />
        </Field>

        <Field label="Poll Interval (seconds)" hint="How often to check for new tasks (min 10)">
          <input
            type="number"
            min={10}
            value={pollInterval}
            onChange={(e) => setPollInterval(Number(e.target.value))}
            className="w-full text-sm rounded px-2 py-1.5 border-none outline-none"
            style={{
              backgroundColor: "var(--flow-bg-tertiary)",
              color: "var(--flow-text-primary)",
            }}
          />
        </Field>

        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={notifications}
            onChange={(e) => setNotifications(e.target.checked)}
            className="rounded"
          />
          <span className="text-sm" style={{ color: "var(--flow-text-primary)" }}>
            Enable notifications
          </span>
        </label>

        <label className="flex items-center gap-2 cursor-pointer">
          <input
            type="checkbox"
            checked={startOnLogin}
            onChange={(e) => setStartOnLogin(e.target.checked)}
            className="rounded"
          />
          <span className="text-sm" style={{ color: "var(--flow-text-primary)" }}>
            Start on login
          </span>
        </label>
      </div>

      {/* Save */}
      <div className="px-3 py-2" style={{ borderTop: "1px solid var(--flow-border)" }}>
        <button
          onClick={handleSave}
          className="w-full text-sm font-medium py-1.5 rounded transition-colors"
          style={{
            backgroundColor: "var(--flow-accent)",
            color: "#ffffff",
          }}
        >
          Save
        </button>
      </div>
    </div>
  );
}

function Field({
  label,
  hint,
  children,
}: {
  label: string;
  hint: string;
  children: React.ReactNode;
}) {
  return (
    <div className="flex flex-col gap-1">
      <label className="text-xs font-medium" style={{ color: "var(--flow-text-secondary)" }}>
        {label}
      </label>
      {children}
      <span className="text-xs" style={{ color: "var(--flow-text-muted)" }}>
        {hint}
      </span>
    </div>
  );
}
