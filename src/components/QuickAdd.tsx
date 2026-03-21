import { useState, useRef, useEffect, useCallback } from "react";
import { parseTaskInput } from "../lib/nlp-parser";
import { useCreateTask } from "../hooks/useTasks";
import { useProjects } from "../hooks/useProjects";
import type { Project } from "../lib/types";

export default function QuickAdd() {
  const [value, setValue] = useState("");
  const [error, setError] = useState<string | null>(null);
  const [selectedProject, setSelectedProject] = useState<Project | null>(null);
  const [showDropdown, setShowDropdown] = useState(false);
  const [dropdownFilter, setDropdownFilter] = useState("");
  const [highlightIndex, setHighlightIndex] = useState(0);
  const inputRef = useRef<HTMLInputElement>(null);
  const dropdownRef = useRef<HTMLDivElement>(null);
  const createTask = useCreateTask();
  const { data: projects } = useProjects();

  // Filter projects for dropdown
  const filteredProjects = (projects ?? []).filter((p) => {
    if (!dropdownFilter) return true;
    return p.name.toLowerCase().includes(dropdownFilter.toLowerCase());
  });

  // Reset highlight when filter changes
  useEffect(() => {
    setHighlightIndex(0);
  }, [dropdownFilter]);

  // Close dropdown on outside click
  useEffect(() => {
    if (!showDropdown) return;
    const handleClick = (e: MouseEvent) => {
      if (dropdownRef.current && !dropdownRef.current.contains(e.target as Node)) {
        setShowDropdown(false);
      }
    };
    document.addEventListener("mousedown", handleClick);
    return () => document.removeEventListener("mousedown", handleClick);
  }, [showDropdown]);

  const selectProject = useCallback((project: Project) => {
    setSelectedProject(project);
    setShowDropdown(false);
    setDropdownFilter("");
    // Remove the #... text from the input
    setValue((prev) => {
      const hashIdx = prev.lastIndexOf("#");
      if (hashIdx === -1) return prev;
      return prev.slice(0, hashIdx).trimEnd();
    });
    inputRef.current?.focus();
  }, []);

  const handleChange = (e: React.ChangeEvent<HTMLInputElement>) => {
    const newValue = e.target.value;
    setValue(newValue);

    // Check for # trigger
    const hashIdx = newValue.lastIndexOf("#");
    if (hashIdx !== -1) {
      const afterHash = newValue.slice(hashIdx + 1);
      // Only trigger if # is at start or preceded by a space
      const charBefore = hashIdx > 0 ? newValue[hashIdx - 1] : " ";
      if (charBefore === " " || hashIdx === 0) {
        setDropdownFilter(afterHash);
        setShowDropdown(true);
        return;
      }
    }
    if (showDropdown && !newValue.includes("#")) {
      setShowDropdown(false);
    }
  };

  const handleKeyDown = (e: React.KeyboardEvent) => {
    if (showDropdown && filteredProjects.length > 0) {
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setHighlightIndex((i) => Math.min(i + 1, filteredProjects.length - 1));
        return;
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setHighlightIndex((i) => Math.max(i - 1, 0));
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        selectProject(filteredProjects[highlightIndex]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setShowDropdown(false);
        return;
      }
    }
  };

  const handleSubmit = (e: React.FormEvent) => {
    e.preventDefault();
    if (showDropdown) return; // Don't submit while picking a project
    const trimmed = value.trim();
    if (!trimmed) return;
    setError(null);

    const parsed = parseTaskInput(trimmed, projects ?? []);

    const input = {
      title: parsed.title,
      priority: parsed.priority || "p4",
      ...(parsed.dueDate ? { dueDate: parsed.dueDate } : {}),
      // Use explicitly selected project first, then NLP-detected project
      ...(selectedProject
        ? { projectId: selectedProject.id }
        : parsed.projectId
          ? { projectId: String(parsed.projectId) }
          : {}),
    };

    createTask.mutate(input, {
      onSuccess: () => {
        setValue("");
        setError(null);
        setSelectedProject(null);
        inputRef.current?.focus();
      },
      onError: (err) => {
        setError((err as Error).message || "Failed to create task");
      },
    });
  };

  return (
    <div style={{ borderBottom: "1px solid var(--flow-border)" }} className="relative">
      <form
        onSubmit={handleSubmit}
        className="flex items-center gap-2 px-3 py-2"
      >
        <span className="text-sm" style={{ color: "var(--flow-accent)" }}>
          &#x2728;
        </span>
        <div className="flex-1 flex items-center gap-1.5 min-w-0">
          {selectedProject && (
            <span
              className="flex items-center gap-1 text-xs px-1.5 py-0.5 rounded flex-shrink-0"
              style={{
                backgroundColor: "var(--flow-accent)",
                color: "#ffffff",
              }}
            >
              {selectedProject.name}
              <button
                type="button"
                onClick={() => setSelectedProject(null)}
                className="hover:opacity-70"
              >
                &#x2715;
              </button>
            </span>
          )}
          <input
            ref={inputRef}
            type="text"
            value={value}
            onChange={handleChange}
            onKeyDown={handleKeyDown}
            placeholder={selectedProject ? "Task title... (p1, today, tomorrow)" : "Task title... (# for project, p1, today)"}
            className="flex-1 text-sm bg-transparent border-none outline-none min-w-0"
            style={{
              color: "var(--flow-text-primary)",
            }}
            disabled={createTask.isPending}
          />
        </div>
        {createTask.isPending && (
          <span className="text-xs" style={{ color: "var(--flow-text-muted)" }}>...</span>
        )}
      </form>

      {/* Project dropdown */}
      {showDropdown && (
        <div
          ref={dropdownRef}
          className="absolute left-3 right-3 z-50 rounded-md shadow-lg overflow-hidden"
          style={{
            backgroundColor: "var(--flow-bg-tertiary)",
            border: "1px solid var(--flow-border)",
            maxHeight: "200px",
            overflowY: "auto",
          }}
        >
          {filteredProjects.length === 0 ? (
            <div
              className="px-3 py-2 text-xs"
              style={{ color: "var(--flow-text-muted)" }}
            >
              No matching projects
            </div>
          ) : (
            filteredProjects.map((project, i) => (
              <button
                key={project.id}
                type="button"
                className="w-full text-left px-3 py-1.5 text-sm transition-colors"
                style={{
                  color: "var(--flow-text-primary)",
                  backgroundColor:
                    i === highlightIndex
                      ? "var(--flow-accent)"
                      : "transparent",
                  ...(i === highlightIndex ? { color: "#ffffff" } : {}),
                }}
                onMouseEnter={() => setHighlightIndex(i)}
                onClick={() => selectProject(project)}
              >
                <span className="mr-1.5" style={{ opacity: 0.6 }}>&#x1F4C1;</span>
                {project.name}
              </button>
            ))
          )}
        </div>
      )}

      {error && (
        <p className="px-3 pb-1 text-xs" style={{ color: "#ef4444" }}>
          {error}
        </p>
      )}
    </div>
  );
}
