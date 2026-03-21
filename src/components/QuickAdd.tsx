import { useState, useRef } from "react";
import { parseTaskInput } from "../lib/nlp-parser";
import { useCreateTask } from "../hooks/useTasks";
import { useProjects } from "../hooks/useProjects";

export default function QuickAdd() {
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const inputRef = useRef<HTMLInputElement>(null);
  const createTask = useCreateTask();
  const { data: projects } = useProjects();

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = value.trim();
    if (!trimmed) return;
    setError(null);

    const parsed = parseTaskInput(trimmed, projects ?? []);

    // Ensure we send the minimum required fields for FLOW
    const input = {
      title: parsed.title,
      priority: parsed.priority || "p4",
      ...(parsed.dueDate ? { dueDate: parsed.dueDate } : {}),
      ...(parsed.projectId ? { projectId: String(parsed.projectId) } : {}),
    };

    createTask.mutate(input, {
      onSuccess: () => {
        setValue("");
        setError(null);
        inputRef.current?.focus();
      },
      onError: (err) => {
        setError((err as Error).message || "Failed to create task");
      },
    });
  };

  return (
    <div style={{ borderBottom: "1px solid var(--flow-border)" }}>
      <form
        onSubmit={handleSubmit}
        className="flex items-center gap-2 px-3 py-2"
      >
        <span className="text-sm" style={{ color: "var(--flow-accent)" }}>
          &#x2728;
        </span>
        <input
          ref={inputRef}
          type="text"
          value={value}
          onChange={(e) => setValue(e.target.value)}
          placeholder="Review contracts for Glastonbury by Friday p1"
          className="flex-1 text-sm bg-transparent border-none outline-none"
          style={{
            color: "var(--flow-text-primary)",
          }}
          disabled={createTask.isPending}
        />
        {createTask.isPending && (
          <span className="text-xs" style={{ color: "var(--flow-text-muted)" }}>...</span>
        )}
      </form>
      {error && (
        <p className="px-3 pb-1 text-xs" style={{ color: "#ef4444" }}>
          {error}
        </p>
      )}
    </div>
  );
}
