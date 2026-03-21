import { useEffect, useRef } from "react";

interface SearchBarProps {
  value: string;
  onChange: (v: string) => void;
  onClose: () => void;
}

export default function SearchBar({ value, onChange, onClose }: SearchBarProps) {
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
  }, []);

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (e.key === "Escape") {
      onClose();
    }
  };

  return (
    <div
      className="flex items-center gap-2 px-3 py-1.5"
      style={{ borderBottom: "1px solid var(--flow-border)" }}
    >
      <span className="text-sm" style={{ color: "var(--flow-text-muted)" }}>
        &#x1F50D;
      </span>
      <input
        ref={inputRef}
        type="text"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        onKeyDown={handleKeyDown}
        placeholder="Search tasks..."
        className="flex-1 text-sm bg-transparent border-none outline-none"
        style={{ color: "var(--flow-text-primary)" }}
      />
      <button
        onClick={onClose}
        className="text-xs"
        style={{ color: "var(--flow-text-muted)" }}
      >
        Esc
      </button>
    </div>
  );
}
