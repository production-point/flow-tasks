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
  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      if (e.key === "Escape" && !isPinned) {
        getCurrentWindow().hide();
      }
    };
    window.addEventListener("keydown", handleKeyDown);
    return () => window.removeEventListener("keydown", handleKeyDown);
  }, [isPinned]);

  const handleDragStart = (e: React.MouseEvent) => {
    if ((e.target as HTMLElement).closest("button")) return;
    getCurrentWindow().startDragging().catch(() => {});
  };

  return (
    <div
      onMouseDown={handleDragStart}
      className="flex items-center justify-between px-3 py-2 select-none cursor-grab active:cursor-grabbing"
      style={{
        backgroundColor: "var(--flow-bg-secondary)",
        borderBottom: "1px solid var(--flow-border)",
      }}
    >
      <div className="flex items-center gap-2">
        <span className="text-sm font-bold" style={{ color: "var(--flow-accent)" }}>
          Flow Tasks
        </span>
        {taskCount > 0 && (
          <span
            className="text-xs font-medium px-1.5 py-0.5 rounded-full leading-none"
            style={{
              backgroundColor: "var(--flow-bg-tertiary)",
              color: "var(--flow-text-secondary)",
              fontSize: "0.65rem",
            }}
          >
            {taskCount}
          </span>
        )}
      </div>

      <div className="flex items-center gap-1">
        <IconButton label="Search" onClick={onToggleSearch}>
          <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor"><path d="M11.742 10.344a6.5 6.5 0 10-1.397 1.398h-.001l3.85 3.85a1 1 0 001.415-1.414l-3.85-3.85-.017.016zm-5.242.656a5 5 0 110-10 5 5 0 010 10z"/></svg>
        </IconButton>
        <IconButton
          label={isPinned ? "Unpin (dock back to menu bar)" : "Pin as floating window"}
          onClick={onTogglePin}
          active={isPinned}
        >
          <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor"><path d="M9.828 1.172a2 2 0 012.828 0l.172.172a2 2 0 010 2.828L11 6l1.5 1.5-5 5L6 11l-1.828 1.828a2 2 0 01-2.828 0l-.172-.172a2 2 0 010-2.828L3 8 1.5 6.5l5-5L8 3l1.828-1.828z"/></svg>
        </IconButton>
        <IconButton label="Settings" onClick={onToggleSettings}>
          <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor"><path d="M8 4.754a3.246 3.246 0 100 6.492 3.246 3.246 0 000-6.492zM5.754 8a2.246 2.246 0 114.492 0 2.246 2.246 0 01-4.492 0z"/><path d="M9.796 1.343c-.527-1.79-3.065-1.79-3.592 0l-.094.319a.873.873 0 01-1.255.52l-.292-.16c-1.64-.892-3.433.902-2.54 2.541l.159.292a.873.873 0 01-.52 1.255l-.319.094c-1.79.527-1.79 3.065 0 3.592l.319.094a.873.873 0 01.52 1.255l-.16.292c-.892 1.64.902 3.434 2.541 2.54l.292-.159a.873.873 0 011.255.52l.094.319c.527 1.79 3.065 1.79 3.592 0l.094-.319a.873.873 0 011.255-.52l.292.16c1.64.893 3.434-.902 2.54-2.541l-.159-.292a.873.873 0 01.52-1.255l.319-.094c1.79-.527 1.79-3.065 0-3.592l-.319-.094a.873.873 0 01-.52-1.255l.16-.292c.893-1.64-.902-3.433-2.541-2.54l-.292.159a.873.873 0 01-1.255-.52l-.094-.319z"/></svg>
        </IconButton>
        <IconButton label="Close" onClick={() => getCurrentWindow().hide()} hoverColor="#d1453b">
          <svg width="14" height="14" viewBox="0 0 16 16" fill="currentColor"><path d="M4.646 4.646a.5.5 0 01.708 0L8 7.293l2.646-2.647a.5.5 0 01.708.708L8.707 8l2.647 2.646a.5.5 0 01-.708.708L8 8.707l-2.646 2.647a.5.5 0 01-.708-.708L7.293 8 4.646 5.354a.5.5 0 010-.708z"/></svg>
        </IconButton>
      </div>
    </div>
  );
}

function IconButton({
  label,
  onClick,
  active,
  hoverColor,
  children,
}: {
  label: string;
  onClick: () => void;
  active?: boolean;
  hoverColor?: string;
  children: React.ReactNode;
}) {
  return (
    <button
      onClick={onClick}
      className="p-1.5 rounded transition-colors hover:bg-[var(--flow-bg-tertiary)]"
      style={{
        color: active ? "var(--flow-accent)" : "var(--flow-text-muted)",
      }}
      onMouseEnter={(e) => { if (hoverColor) (e.currentTarget.style.color = hoverColor); }}
      onMouseLeave={(e) => { if (hoverColor) (e.currentTarget.style.color = "var(--flow-text-muted)"); }}
      aria-label={label}
      title={label}
    >
      {children}
    </button>
  );
}
