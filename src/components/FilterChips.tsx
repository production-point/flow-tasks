export type Filter = "all" | "today" | "overdue" | "p1" | "p2" | "p3";

const FILTERS: { key: Filter; label: string; color?: string }[] = [
  { key: "all", label: "All" },
  { key: "today", label: "Today", color: "#058527" },
  { key: "overdue", label: "Overdue", color: "#d1453b" },
  { key: "p1", label: "P1", color: "#d1453b" },
  { key: "p2", label: "P2", color: "#eb8909" },
  { key: "p3", label: "P3", color: "#3972C5" },
];

interface FilterChipsProps {
  active: Filter;
  onChange: (f: Filter) => void;
  overdueBadge?: number;
}

export default function FilterChips({ active, onChange, overdueBadge }: FilterChipsProps) {
  return (
    <div
      className="flex items-center gap-1 px-3 py-1.5"
      style={{ borderBottom: "1px solid var(--flow-border)" }}
    >
      {FILTERS.map((f) => {
        const isActive = active === f.key;
        return (
          <button
            key={f.key}
            onClick={() => onChange(f.key)}
            className="text-xs px-2 py-0.5 rounded-full transition-colors"
            style={{
              backgroundColor: isActive ? (f.color ?? "var(--flow-accent)") : "transparent",
              color: isActive ? "#fff" : "var(--flow-text-secondary)",
              fontWeight: isActive ? 600 : 400,
            }}
          >
            {f.label}
            {f.key === "overdue" && overdueBadge ? ` (${overdueBadge})` : ""}
          </button>
        );
      })}
    </div>
  );
}
