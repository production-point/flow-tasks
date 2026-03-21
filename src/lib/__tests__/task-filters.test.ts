import { describe, it, expect } from "vitest";
import { toDateString } from "../date-utils";

function filterTasks(
  tasks: Array<{ id: number; title: string; priority: string | null; dueDate: string | null; completed: boolean; parentTaskId: number | null }>,
  filter: string,
  today: string
) {
  let result = tasks.filter((t) => !t.completed && !t.parentTaskId);
  if (filter === "today") result = result.filter((t) => toDateString(t.dueDate) === today);
  if (filter === "overdue") {
    result = result.filter((t) => {
      const d = toDateString(t.dueDate);
      return d !== null && d < today;
    });
  }
  if (filter === "p1") result = result.filter((t) => t.priority === "p1");
  if (filter === "p2") result = result.filter((t) => t.priority === "p2");
  if (filter === "p3") result = result.filter((t) => t.priority === "p3");
  return result;
}

function sortTasks(
  tasks: Array<{ priority: string | null; dueDate: string | null }>,
  today: string
) {
  return [...tasks].sort((a, b) => {
    const aDate = toDateString(a.dueDate);
    const bDate = toDateString(b.dueDate);
    const aOverdue = aDate && aDate < today ? 0 : 1;
    const bOverdue = bDate && bDate < today ? 0 : 1;
    if (aOverdue !== bOverdue) return aOverdue - bOverdue;
    const order: Record<string, number> = { p1: 0, p2: 1, p3: 2, p4: 3 };
    const aPri = order[a.priority || "p4"] ?? 3;
    const bPri = order[b.priority || "p4"] ?? 3;
    if (aPri !== bPri) return aPri - bPri;
    if (aDate && bDate) return aDate.localeCompare(bDate);
    if (aDate) return -1;
    if (bDate) return 1;
    return 0;
  });
}

const today = "2026-03-21";

const tasks = [
  { id: 1, title: "Overdue P2", priority: "p2", dueDate: "2026-03-19", completed: false, parentTaskId: null },
  { id: 2, title: "Today P1", priority: "p1", dueDate: "2026-03-21", completed: false, parentTaskId: null },
  { id: 3, title: "Future P3", priority: "p3", dueDate: "2026-03-25", completed: false, parentTaskId: null },
  { id: 4, title: "Completed", priority: "p1", dueDate: "2026-03-21", completed: true, parentTaskId: null },
  { id: 5, title: "Subtask", priority: "p1", dueDate: null, completed: false, parentTaskId: 2 },
  { id: 6, title: "No date P1", priority: "p1", dueDate: null, completed: false, parentTaskId: null },
];

describe("filterTasks", () => {
  it("filters out completed and subtasks by default", () => {
    const result = filterTasks(tasks, "all", today);
    expect(result.map((t) => t.id)).toEqual([1, 2, 3, 6]);
  });

  it("filters to today only", () => {
    const result = filterTasks(tasks, "today", today);
    expect(result.map((t) => t.id)).toEqual([2]);
  });

  it("filters to overdue only", () => {
    const result = filterTasks(tasks, "overdue", today);
    expect(result.map((t) => t.id)).toEqual([1]);
  });

  it("filters by priority", () => {
    const result = filterTasks(tasks, "p1", today);
    expect(result.map((t) => t.id)).toEqual([2, 6]);
  });

  it("handles ISO timestamp dates correctly", () => {
    const isoTasks = [
      { id: 10, title: "ISO overdue", priority: "p1", dueDate: "2026-03-19T07:20:22.296Z", completed: false, parentTaskId: null },
      { id: 11, title: "ISO today", priority: "p1", dueDate: "2026-03-21T14:00:00.000Z", completed: false, parentTaskId: null },
    ];
    const overdue = filterTasks(isoTasks, "overdue", today);
    expect(overdue.map((t) => t.id)).toEqual([10]);
    const todayTasks = filterTasks(isoTasks, "today", today);
    expect(todayTasks.map((t) => t.id)).toEqual([11]);
  });
});

describe("toDateString", () => {
  it("normalizes ISO timestamps to YYYY-MM-DD", () => {
    expect(toDateString("2026-03-19T07:20:22.296Z")).toBe("2026-03-19");
  });

  it("passes through YYYY-MM-DD strings", () => {
    expect(toDateString("2026-03-19")).toBe("2026-03-19");
  });

  it("returns null for null input", () => {
    expect(toDateString(null)).toBeNull();
  });
});

describe("sortTasks", () => {
  it("puts overdue first, then by priority, then by date", () => {
    const input = [
      { priority: "p3", dueDate: "2026-03-25" },
      { priority: "p1", dueDate: "2026-03-21" },
      { priority: "p2", dueDate: "2026-03-19" },
      { priority: "p1", dueDate: null },
    ];
    const result = sortTasks(input, today);
    expect(result.map((t) => t.dueDate)).toEqual(["2026-03-19", "2026-03-21", null, "2026-03-25"]);
  });

  it("sorts ISO timestamp dates correctly", () => {
    const input = [
      { priority: "p1", dueDate: "2026-03-25T10:00:00.000Z" },
      { priority: "p1", dueDate: "2026-03-19T07:20:22.296Z" },
    ];
    const result = sortTasks(input, today);
    expect(result.map((t) => t.dueDate)).toEqual([
      "2026-03-19T07:20:22.296Z",
      "2026-03-25T10:00:00.000Z",
    ]);
  });
});
