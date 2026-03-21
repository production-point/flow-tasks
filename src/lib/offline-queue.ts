import type { UpdateTaskInput, CreateTaskInput } from "./types";

interface QueuedMutation {
  id: string;
  type: "create" | "update";
  taskId?: string;
  input: CreateTaskInput | UpdateTaskInput;
  timestamp: number;
}

const STORAGE_KEY = "flow-tasks-offline-queue";

export function getQueue(): QueuedMutation[] {
  try {
    return JSON.parse(localStorage.getItem(STORAGE_KEY) || "[]");
  } catch {
    return [];
  }
}

export function enqueue(mutation: Omit<QueuedMutation, "id" | "timestamp">): void {
  const queue = getQueue();
  queue.push({ ...mutation, id: crypto.randomUUID(), timestamp: Date.now() });
  localStorage.setItem(STORAGE_KEY, JSON.stringify(queue));
}

export function dequeue(id: string): void {
  const queue = getQueue().filter((m) => m.id !== id);
  localStorage.setItem(STORAGE_KEY, JSON.stringify(queue));
}

export function clearQueue(): void {
  localStorage.removeItem(STORAGE_KEY);
}
