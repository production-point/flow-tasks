import { useMemo, useState } from "react";
import { useTasks, useCompleteTask } from "../hooks/useTasks";
import { useProjects } from "../hooks/useProjects";
import { useLabels } from "../hooks/useLabels";
import { useSettings } from "../hooks/useSettings";
import type { Task } from "../lib/types";
import TaskRow from "./TaskRow";
import QuickAdd from "./QuickAdd";
import FilterChips, { type Filter } from "./FilterChips";
import OfflineIndicator from "./OfflineIndicator";

interface TaskListProps {
  searchQuery: string;
  onEditTask: (task: Task) => void;
}

type TabKey = "all" | "by-project" | "overdue";

const PRIORITY_ORDER: Record<string, number> = { p1: 0, p2: 1, p3: 2, p4: 3 };

function isOverdue(task: Task): boolean {
  if (!task.dueDate || task.completed) return false;
  const today = new Date();
  today.setHours(0, 0, 0, 0);
  return new Date(task.dueDate + "T00:00:00") < today;
}

function isToday(task: Task): boolean {
  if (!task.dueDate) return false;
  return task.dueDate === new Date().toISOString().split("T")[0];
}

function sortTasks(a: Task, b: Task): number {
  // Overdue first
  const aOverdue = isOverdue(a) ? 0 : 1;
  const bOverdue = isOverdue(b) ? 0 : 1;
  if (aOverdue !== bOverdue) return aOverdue - bOverdue;

  // Then by priority
  const aPri = PRIORITY_ORDER[a.priority ?? "p4"] ?? 3;
  const bPri = PRIORITY_ORDER[b.priority ?? "p4"] ?? 3;
  if (aPri !== bPri) return aPri - bPri;

  // Then by due date
  if (a.dueDate && b.dueDate) return a.dueDate.localeCompare(b.dueDate);
  if (a.dueDate) return -1;
  if (b.dueDate) return 1;

  return 0;
}

function applyFilter(tasks: Task[], filter: Filter): Task[] {
  switch (filter) {
    case "today":
      return tasks.filter((t) => isToday(t));
    case "overdue":
      return tasks.filter((t) => isOverdue(t));
    case "p1":
      return tasks.filter((t) => t.priority === "p1");
    case "p2":
      return tasks.filter((t) => t.priority === "p2");
    case "p3":
      return tasks.filter((t) => t.priority === "p3");
    default:
      return tasks;
  }
}

function applySearch(tasks: Task[], query: string): Task[] {
  if (!query) return tasks;
  const lower = query.toLowerCase();
  return tasks.filter((t) => t.title.toLowerCase().includes(lower));
}

export function useActiveTaskCount(): number {
  const { data: tasks } = useTasks();
  return useMemo(
    () => (tasks ?? []).filter((t) => !t.completed && !t.parentTaskId).length,
    [tasks]
  );
}

export default function TaskList({ searchQuery, onEditTask }: TaskListProps) {
  const { data: tasks, isLoading, isError } = useTasks();
  const { data: projects } = useProjects();
  const { data: labels } = useLabels();
  const completeTask = useCompleteTask();
  const activeTab = useSettings((s) => s.settings.activeTab);
  const updateSettings = useSettings((s) => s.updateSettings);
  const [filter, setFilter] = useState<Filter>("all");

  const setTab = (tab: TabKey) => updateSettings({ activeTab: tab });

  const projectMap = useMemo(() => {
    const map = new Map<string, (typeof projects extends (infer T)[] | undefined ? T : never)>();
    projects?.forEach((p) => map.set(p.id, p));
    return map;
  }, [projects]);

  // Base filtered tasks: exclude completed and subtasks from main list
  const baseTasks = useMemo(() => {
    if (!tasks) return [];
    return tasks.filter((t) => !t.completed && !t.parentTaskId);
  }, [tasks]);

  // Subtasks map keyed by parentTaskId
  const subtaskMap = useMemo(() => {
    const map = new Map<string, Task[]>();
    if (!tasks) return map;
    tasks
      .filter((t) => t.parentTaskId && !t.completed)
      .forEach((t) => {
        const list = map.get(t.parentTaskId!) ?? [];
        list.push(t);
        map.set(t.parentTaskId!, list);
      });
    return map;
  }, [tasks]);

  const overdueCount = useMemo(
    () => baseTasks.filter((t) => isOverdue(t)).length,
    [baseTasks]
  );

  // Apply search + filter + sort
  const filteredTasks = useMemo(() => {
    let result = applySearch(baseTasks, searchQuery);
    if (activeTab === "overdue") {
      result = result.filter((t) => isOverdue(t));
    } else if (activeTab === "all") {
      result = applyFilter(result, filter);
    }
    return [...result].sort(sortTasks);
  }, [baseTasks, searchQuery, activeTab, filter]);

  // Group by project
  const groupedByProject = useMemo(() => {
    const groups = new Map<string | null, Task[]>();
    filteredTasks.forEach((t) => {
      const key = t.projectId;
      const list = groups.get(key) ?? [];
      list.push(t);
      groups.set(key, list);
    });
    return groups;
  }, [filteredTasks]);

  const handleComplete = (id: string) => completeTask.mutate(id);

  const renderTask = (task: Task, isSub = false) => (
    <div key={task.id}>
      <TaskRow
        task={task}
        project={task.projectId ? projectMap.get(task.projectId) : undefined}
        labels={labels}
        isSubtask={isSub}
        onComplete={handleComplete}
        onClick={onEditTask}
      />
      {subtaskMap.get(task.id)?.map((sub) => renderTask(sub, true))}
    </div>
  );

  if (isLoading) {
    return (
      <div className="flex items-center justify-center py-12 text-sm" style={{ color: "var(--flow-text-muted)" }}>
        Loading...
      </div>
    );
  }

  const TABS: { key: TabKey; label: string }[] = [
    { key: "all", label: "All" },
    { key: "by-project", label: "By Project" },
    { key: "overdue", label: "Overdue" },
  ];

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {isError && <OfflineIndicator />}
      <QuickAdd />

      {/* Tabs */}
      <div
        className="flex items-center gap-0"
        style={{ borderBottom: "1px solid var(--flow-border)" }}
      >
        {TABS.map(({ key, label }) => (
          <button
            key={key}
            onClick={() => setTab(key)}
            className="flex-1 text-xs py-1.5 text-center transition-colors"
            style={{
              color: activeTab === key ? "var(--flow-accent)" : "var(--flow-text-muted)",
              borderBottom: activeTab === key ? "2px solid var(--flow-accent)" : "2px solid transparent",
            }}
          >
            {label}
          </button>
        ))}
      </div>

      {/* Filter chips (only in All tab) */}
      {activeTab === "all" && (
        <FilterChips active={filter} onChange={setFilter} overdueBadge={overdueCount} />
      )}

      {/* Task list */}
      <div className="flex-1 overflow-y-auto">
        {filteredTasks.length === 0 ? (
          <div className="flex items-center justify-center py-12 text-sm" style={{ color: "var(--flow-text-muted)" }}>
            {baseTasks.length === 0 ? "No tasks \u2014 nice work! \uD83C\uDF89" : "No matching tasks"}
          </div>
        ) : activeTab === "by-project" ? (
          <ProjectGroupedList
            groups={groupedByProject}
            projectMap={projectMap}
            renderTask={renderTask}
          />
        ) : (
          filteredTasks.map((t) => renderTask(t))
        )}
      </div>
    </div>
  );
}

function ProjectGroupedList({
  groups,
  projectMap,
  renderTask,
}: {
  groups: Map<string | null, Task[]>;
  projectMap: Map<string, { id: string; name: string; color: string | null; status: string | null }>;
  renderTask: (task: Task, isSub?: boolean) => React.ReactNode;
}) {
  const [collapsed, setCollapsed] = useState<Set<string | null>>(new Set());

  const toggle = (key: string | null) => {
    setCollapsed((prev) => {
      const next = new Set(prev);
      if (next.has(key)) next.delete(key);
      else next.add(key);
      return next;
    });
  };

  const entries = Array.from(groups.entries()).sort(([a], [b]) => {
    // Projects first, then ungrouped
    if (a === null) return 1;
    if (b === null) return -1;
    const nameA = projectMap.get(a)?.name ?? "";
    const nameB = projectMap.get(b)?.name ?? "";
    return nameA.localeCompare(nameB);
  });

  return (
    <>
      {entries.map(([projectId, tasks]) => {
        const project = projectId !== null ? projectMap.get(projectId) : null;
        const isCollapsed = collapsed.has(projectId);

        return (
          <div key={projectId ?? "none"}>
            <button
              onClick={() => toggle(projectId)}
              className="flex items-center gap-1.5 w-full text-left px-3 py-1.5 text-xs font-medium"
              style={{
                backgroundColor: "var(--flow-bg-secondary)",
                color: "var(--flow-text-secondary)",
              }}
            >
              <span>{isCollapsed ? "\u25B6" : "\u25BC"}</span>
              <span>{project ? `\uD83D\uDCC1 ${project.name}` : "No Project"}</span>
              <span style={{ color: "var(--flow-text-muted)" }}>({tasks.length})</span>
            </button>
            {!isCollapsed && tasks.map((t) => renderTask(t))}
          </div>
        );
      })}
    </>
  );
}
