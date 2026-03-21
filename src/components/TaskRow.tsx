import type { Task, Project, Label } from "../lib/types";
import { toDateString } from "../lib/date-utils";

const PRIORITY_COLORS: Record<string, string> = {
  p1: "#d1453b",
  p2: "#eb8909",
  p3: "#3972C5",
  p4: "#999",
};

function getDueBadge(dueDate: string | null): {
  text: string;
  color: string;
} | null {
  const dateStr = toDateString(dueDate);
  if (!dateStr) return null;
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  const due = new Date(dateStr + "T00:00:00");
  const diffMs = due.getTime() - today.getTime();
  const diffDays = Math.round(diffMs / 86_400_000);

  if (diffDays < 0) return { text: `${Math.abs(diffDays)}d overdue`, color: "#d1453b" };
  if (diffDays === 0) return { text: "Today", color: "#058527" };
  if (diffDays === 1) return { text: "Tomorrow", color: "#ad6200" };
  if (diffDays <= 7) {
    const dayName = due.toLocaleDateString("en-US", { weekday: "short" });
    return { text: dayName, color: "#692fc2" };
  }
  return { text: due.toLocaleDateString("en-GB", { day: "numeric", month: "short" }), color: "#666" };
}

function isOverdue(dueDate: string | null): boolean {
  const dateStr = toDateString(dueDate);
  if (!dateStr) return false;
  const today = new Date().toISOString().split("T")[0];
  return dateStr < today;
}

interface TaskRowProps {
  task: Task;
  project?: Project;
  labels?: Label[];
  isSubtask?: boolean;
  onComplete: (id: string) => void;
  onClick: (task: Task) => void;
}

export default function TaskRow({
  task,
  project,
  labels,
  isSubtask,
  onComplete,
  onClick,
}: TaskRowProps) {
  const priorityColor = task.priority ? PRIORITY_COLORS[task.priority] ?? "#999" : "#999";
  const overdue = !task.completed && isOverdue(task.dueDate);
  const dueBadge = getDueBadge(task.dueDate);
  const taskLabels = labels?.filter((l) => task.labels?.includes(l.id)) ?? [];

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onClick(task)}
      onKeyDown={(e) => { if (e.key === "Enter") onClick(task); }}
      className="group cursor-pointer transition-colors hover:bg-[#fafafa]"
      style={{
        paddingLeft: isSubtask ? "2.5rem" : "0.75rem",
        paddingRight: "0.75rem",
        paddingTop: "0.5rem",
        paddingBottom: "0.5rem",
        borderBottom: "1px solid var(--flow-border)",
        backgroundColor: overdue ? "#fef2f2" : undefined,
      }}
    >
      {/* Row 1: checkbox + title + project */}
      <div className="flex items-start gap-2.5">
        <button
          onClick={(e) => {
            e.stopPropagation();
            onComplete(task.id);
          }}
          className="flex-shrink-0 mt-0.5 rounded-full border-2 transition-colors hover:bg-opacity-20"
          style={{
            width: 18,
            height: 18,
            borderColor: priorityColor,
            backgroundColor: task.completed ? priorityColor : "transparent",
          }}
          aria-label={task.completed ? "Completed" : "Mark complete"}
        >
          {task.completed && (
            <svg viewBox="0 0 18 18" fill="none" style={{ width: 14, height: 14, margin: "auto", display: "block" }}>
              <path d="M5 9l3 3 5-5" stroke="#fff" strokeWidth="2" strokeLinecap="round" strokeLinejoin="round" />
            </svg>
          )}
        </button>
        <span
          className="flex-1 text-sm leading-snug"
          style={{
            color: task.completed ? "var(--flow-text-muted)" : "var(--flow-text-primary)",
            textDecoration: task.completed ? "line-through" : undefined,
          }}
        >
          {task.title}
        </span>
        {project && (
          <span className="text-xs flex items-center gap-0.5 flex-shrink-0" style={{ color: "var(--flow-text-muted)" }}>
            <svg width="10" height="10" viewBox="0 0 16 16" fill="currentColor"><circle cx="8" cy="8" r="3"/></svg>
            {project.name}
          </span>
        )}
      </div>

      {/* Row 2: metadata */}
      {(dueBadge || taskLabels.length > 0 || task.isRecurring || (task.priority && task.priority !== "p4")) && (
        <div className="flex items-center gap-2 mt-1 flex-wrap" style={{ marginLeft: "1.75rem" }}>
          {dueBadge && (
            <span className="text-xs flex items-center gap-0.5" style={{ color: dueBadge.color }}>
              <svg width="12" height="12" viewBox="0 0 16 16" fill="currentColor"><path d="M5 1v2H3.5A1.5 1.5 0 002 4.5v9A1.5 1.5 0 003.5 15h9a1.5 1.5 0 001.5-1.5v-9A1.5 1.5 0 0012.5 3H11V1h-1v2H6V1H5zm-1.5 4h9v8.5h-9V5z"/></svg>
              {dueBadge.text}
            </span>
          )}
          {taskLabels.map((label) => (
            <span
              key={label.id}
              className="text-xs px-1.5 py-0 rounded-sm"
              style={{
                backgroundColor: label.color ? `${label.color}18` : "#f0f0f0",
                color: label.color ?? "var(--flow-text-secondary)",
              }}
            >
              {label.name}
            </span>
          ))}
          {task.isRecurring && (
            <span className="text-xs" style={{ color: "var(--flow-text-muted)" }} title="Recurring">
              &#x1F501;
            </span>
          )}
        </div>
      )}
    </div>
  );
}
