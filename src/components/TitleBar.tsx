import { useEffect } from "react";
import { getCurrentWindow } from "@tauri-apps/api/window";

interface TitleBarProps {
  taskCount: number;
  isPinned: boolean;
  onTogglePin: () => void;
  onToggleSettings: () => void;
  onToggleSearch: () => void;
}

export default function TitleBar({
  taskCount,
  isPinned,
  onTogglePin,
  onToggleSettings,
  onToggleSearch,
}: TitleBarProps) {
  // Global Esc handler: hide window when not pinned
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !isPinned) {
        getCurrentWindow().hide();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isPinned]);

  return (
    <div
      data-tauri-drag-region
      className="flex items-center justify-between px-3 py-1.5 select-none"
      style={{
        backgroundColor: "var(--flow-bg-secondary)",
        borderBottom: "1px solid var(--flow-border)",
      }}
    >
      {/* Left: title + count */}
      <div data-tauri-drag-region className="flex items-center gap-2">
        <span
          data-tauri-drag-region
          className="text-xs font-semibold"
          style={{ color: "var(--flow-text-primary)" }}
        >
          Flow Tasks
        </span>
        <span
          className="text-xs font-medium px-1.5 py-0.5 rounded-full leading-none"
          style={{
            backgroundColor: "var(--flow-accent)",
            color: "#ffffff",
            fontSize: "0.65rem",
            minWidth: "1.25rem",
            textAlign: "center",
          }}
        >
          {taskCount}
        </span>
      </div>

      {/* Right: action buttons */}
      <div className="flex items-center gap-0.5">
        <TitleButton
          label="Search"
          onClick={onToggleSearch}
          icon={"\uD83D\uDD0D"}
        />
        <TitleButton
          label="Pin"
          onClick={onTogglePin}
          icon={"\uD83D\uDCCC"}
          active={isPinned}
        />
        <TitleButton
          label="Settings"
          onClick={onToggleSettings}
          icon={"\u2699\uFE0F"}
        />
        <button
          onClick={() => getCurrentWindow().hide()}
          className="text-xs px-1.5 py-0.5 rounded transition-colors hover:bg-red-500/20 hover:text-red-400"
          style={{ color: "var(--flow-text-muted)" }}
          aria-label="Close"
        >
          &#x2715;
        </button>
      </div>
    </div>
  );
}

function TitleButton({
  label,
  onClick,
  icon,
  active,
}: {
  label: string;
  onClick: () => void;
  icon: string;
  active?: boolean;
}) {
  return (
    <button
      onClick={onClick}
      className="text-xs px-1.5 py-0.5 rounded transition-colors"
      style={{
        color: active ? "var(--flow-accent)" : "var(--flow-text-muted)",
      }}
      aria-label={label}
    >
      {icon}
    </button>
  );
}
