import { useState } from "react";
import { useUpdateTask } from "../hooks/useTasks";
import { useProjects } from "../hooks/useProjects";
import { useUsers } from "../hooks/useUsers";
import type { Task } from "../lib/types";

interface TaskEditorProps {
  task: Task;
  onClose: () => void;
}

const PRIORITIES = ["p1", "p2", "p3", "p4"] as const;

const PRIORITY_COLORS: Record<string, string> = {
  p1: "#ef4444",
  p2: "#f97316",
  p3: "#3b82f6",
  p4: "var(--flow-text-muted)",
};

export default function TaskEditor({ task, onClose }: TaskEditorProps) {
  const [title, setTitle] = useState(task.title);
  const [priority, setPriority] = useState<Task["priority"]>(task.priority);
  const [dueDate, setDueDate] = useState(task.dueDate ?? "");
  const [projectId, setProjectId] = useState<string | null>(task.projectId);
  const [assigneeId, setAssigneeId] = useState<string | null>(task.assigneeId);

  const updateTask = useUpdateTask();
  const { data: projects } = useProjects();
  const { data: users } = useUsers();

  const handleSave = () => {
    updateTask.mutate(
      {
        id: task.id,
        input: {
          title,
          priority,
          dueDate: dueDate || null,
          projectId,
          assigneeId,
        },
      },
      { onSuccess: onClose }
    );
  };

  return (
    <div className="flex flex-col h-full">
      {/* Header */}
      <div
        className="flex items-center justify-between px-3 py-2"
        style={{ borderBottom: "1px solid var(--flow-border)" }}
      >
        <span className="text-sm font-semibold" style={{ color: "var(--flow-text-primary)" }}>
          Edit Task
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
        {/* Title */}
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium" style={{ color: "var(--flow-text-secondary)" }}>
            Title
          </label>
          <input
            type="text"
            value={title}
            onChange={(e) => setTitle(e.target.value)}
            className="w-full text-sm rounded px-2 py-1.5 border-none outline-none"
            style={{
              backgroundColor: "var(--flow-bg-tertiary)",
              color: "var(--flow-text-primary)",
            }}
          />
        </div>

        {/* Priority */}
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium" style={{ color: "var(--flow-text-secondary)" }}>
            Priority
          </label>
          <div className="flex gap-1">
            {PRIORITIES.map((p) => (
              <button
                key={p}
                onClick={() => setPriority(priority === p ? null : p)}
                className="flex-1 text-xs py-1.5 rounded font-medium transition-colors"
                style={{
                  backgroundColor: priority === p ? `${PRIORITY_COLORS[p]}20` : "var(--flow-bg-tertiary)",
                  color: priority === p ? PRIORITY_COLORS[p] : "var(--flow-text-muted)",
                  border: priority === p ? `1px solid ${PRIORITY_COLORS[p]}40` : "1px solid transparent",
                }}
              >
                {p.toUpperCase()}
              </button>
            ))}
          </div>
        </div>

        {/* Due Date */}
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium" style={{ color: "var(--flow-text-secondary)" }}>
            Due Date
          </label>
          <input
            type="date"
            value={dueDate}
            onChange={(e) => setDueDate(e.target.value)}
            className="w-full text-sm rounded px-2 py-1.5 border-none outline-none"
            style={{
              backgroundColor: "var(--flow-bg-tertiary)",
              color: "var(--flow-text-primary)",
              colorScheme: "dark",
            }}
          />
        </div>

        {/* Project */}
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium" style={{ color: "var(--flow-text-secondary)" }}>
            Project
          </label>
          <select
            value={projectId ?? ""}
            onChange={(e) => setProjectId(e.target.value || null)}
            className="w-full text-sm rounded px-2 py-1.5 border-none outline-none"
            style={{
              backgroundColor: "var(--flow-bg-tertiary)",
              color: "var(--flow-text-primary)",
            }}
          >
            <option value="">None</option>
            {projects?.map((p) => (
              <option key={p.id} value={p.id}>
                {p.name}
              </option>
            ))}
          </select>
        </div>

        {/* Assignee */}
        <div className="flex flex-col gap-1">
          <label className="text-xs font-medium" style={{ color: "var(--flow-text-secondary)" }}>
            Assignee
          </label>
          <select
            value={assigneeId ?? ""}
            onChange={(e) => setAssigneeId(e.target.value || null)}
            className="w-full text-sm rounded px-2 py-1.5 border-none outline-none"
            style={{
              backgroundColor: "var(--flow-bg-tertiary)",
              color: "var(--flow-text-primary)",
            }}
          >
            <option value="">Unassigned</option>
            {users?.map((u) => (
              <option key={u.id} value={u.id}>
                {[u.firstName, u.lastName].filter(Boolean).join(" ") || u.email || `User ${u.id}`}
              </option>
            ))}
          </select>
        </div>
      </div>

      {/* Save */}
      <div className="px-3 py-2" style={{ borderTop: "1px solid var(--flow-border)" }}>
        <button
          onClick={handleSave}
          disabled={updateTask.isPending}
          className="w-full text-sm font-medium py-1.5 rounded transition-colors"
          style={{
            backgroundColor: "var(--flow-accent)",
            color: "#ffffff",
            opacity: updateTask.isPending ? 0.6 : 1,
          }}
        >
          {updateTask.isPending ? "Saving..." : "Save"}
        </button>
      </div>
    </div>
  );
}
