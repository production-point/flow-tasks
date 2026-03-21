export type Filter = "all" | "today" | "overdue" | "p1" | "p2" | "p3";

interface FilterChipsProps {
  active: Filter;
  onChange: (f: Filter) => void;
  overdueBadge?: number;
}

const FILTERS: { key: Filter; label: string }[] = [
  { key: "all", label: "All" },
  { key: "today", label: "Today" },
  { key: "overdue", label: "Overdue" },
  { key: "p1", label: "P1" },
  { key: "p2", label: "P2" },
  { key: "p3", label: "P3" },
];

export default function FilterChips({ active, onChange, overdueBadge }: FilterChipsProps) {
  return (
    <div className="flex items-center gap-1.5 px-3 py-1.5 overflow-x-auto">
      {FILTERS.map(({ key, label }) => {
        const isActive = active === key;
        return (
          <button
            key={key}
            onClick={() => onChange(key)}
            className="text-xs px-2.5 py-1 rounded-full whitespace-nowrap transition-colors flex items-center gap-1"
            style={{
              backgroundColor: isActive ? "var(--flow-accent)" : "var(--flow-bg-tertiary)",
              color: isActive ? "#ffffff" : "var(--flow-text-secondary)",
            }}
          >
            {label}
            {key === "overdue" && overdueBadge !== undefined && overdueBadge > 0 && (
              <span
                className="text-xs font-medium px-1 py-0.5 rounded-full leading-none"
                style={{
                  backgroundColor: isActive ? "rgba(255,255,255,0.2)" : "#ef4444",
                  color: "#ffffff",
                  fontSize: "0.65rem",
                  minWidth: "1rem",
                  textAlign: "center",
                }}
              >
                {overdueBadge}
              </span>
            )}
          </button>
        );
      })}
    </div>
  );
}
