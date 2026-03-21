import type { Task, Project, Label } from "../lib/types";
import { toDateString } from "../lib/date-utils";

const PRIORITY_COLORS: Record<string, string> = {
  p1: "#ef4444",
  p2: "#f97316",
  p3: "#3b82f6",
};

const PRIORITY_LABELS: Record<string, string> = {
  p1: "P1",
  p2: "P2",
  p3: "P3",
  p4: "P4",
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

  if (diffDays < 0) return { text: `${Math.abs(diffDays)}d overdue`, color: "#ef4444" };
  if (diffDays === 0) return { text: "Today", color: "#f59e0b" };
  if (diffDays === 1) return { text: "Tomorrow", color: "#94a3b8" };
  return { text: due.toLocaleDateString("en-GB", { day: "numeric", month: "short" }), color: "#94a3b8" };
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
  const borderColor = task.priority ? PRIORITY_COLORS[task.priority] ?? "transparent" : "transparent";
  const overdue = !task.completed && isOverdue(task.dueDate);
  const dueBadge = getDueBadge(task.dueDate);
  const taskLabels = labels?.filter((l) => task.labels?.includes(l.id)) ?? [];

  return (
    <div
      role="button"
      tabIndex={0}
      onClick={() => onClick(task)}
      onKeyDown={(e) => { if (e.key === "Enter") onClick(task); }}
      className="flex flex-col gap-1 px-3 py-2 cursor-pointer transition-colors"
      style={{
        borderLeft: `3px solid ${borderColor}`,
        paddingLeft: isSubtask ? "2rem" : undefined,
        backgroundColor: overdue ? "rgba(239, 68, 68, 0.05)" : undefined,
      }}
    >
      {/* Row 1: checkbox + title + project badge */}
      <div className="flex items-center gap-2">
        <button
          onClick={(e) => {
            e.stopPropagation();
            onComplete(task.id);
          }}
          className="flex-shrink-0 rounded-sm border transition-colors"
          style={{
            width: 14,
            height: 14,
            borderColor: task.completed ? "var(--flow-accent)" : "var(--flow-text-muted)",
            backgroundColor: task.completed ? "var(--flow-accent)" : "transparent",
          }}
          aria-label={task.completed ? "Completed" : "Mark complete"}
        />
        <span
          className="flex-1 text-sm truncate"
          style={{
            color: task.completed ? "var(--flow-text-muted)" : "var(--flow-text-primary)",
            textDecoration: task.completed ? "line-through" : undefined,
          }}
        >
          {task.title}
        </span>
        {project && (
          <span
            className="text-xs px-1.5 py-0.5 rounded flex-shrink-0"
            style={{
              backgroundColor: "var(--flow-bg-tertiary)",
              color: "var(--flow-text-secondary)",
            }}
          >
            {project.name}
          </span>
        )}
      </div>

      {/* Row 2: priority badge, due badge, labels, recurring icon */}
      <div className="flex items-center gap-1.5 ml-5">
        {task.priority && (
          <span
            className="text-xs px-1 py-0.5 rounded font-medium"
            style={{
              backgroundColor: `${PRIORITY_COLORS[task.priority] ?? "var(--flow-bg-tertiary)"}20`,
              color: PRIORITY_COLORS[task.priority] ?? "var(--flow-text-secondary)",
            }}
          >
            {PRIORITY_LABELS[task.priority]}
          </span>
        )}
        {dueBadge && (
          <span
            className="text-xs px-1 py-0.5 rounded"
            style={{ color: dueBadge.color }}
          >
            {dueBadge.text}
          </span>
        )}
        {taskLabels.map((label) => (
          <span
            key={label.id}
            className="text-xs px-1 py-0.5 rounded"
            style={{
              backgroundColor: label.color ? `${label.color}20` : "var(--flow-bg-tertiary)",
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
    </div>
  );
}
