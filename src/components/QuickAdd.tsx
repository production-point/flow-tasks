import { useState, useRef } from "react";
import { parseTaskInput } from "../lib/nlp-parser";
import { useCreateTask } from "../hooks/useTasks";
import { useProjects } from "../hooks/useProjects";

export default function QuickAdd() {
  const [value, setValue] = useState("");
  const inputRef = useRef<HTMLInputElement>(null);
  const createTask = useCreateTask();
  const { data: projects } = useProjects();

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    const trimmed = value.trim();
    if (!trimmed) return;

    const input = parseTaskInput(trimmed, projects ?? []);
    createTask.mutate(input, {
      onSuccess: () => {
        setValue("");
        inputRef.current?.focus();
      },
    });
  };

  return (
    <form
      onSubmit={handleSubmit}
      className="flex items-center gap-2 px-3 py-2"
      style={{ borderBottom: "1px solid var(--flow-border)" }}
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
      />
    </form>
  );
}
