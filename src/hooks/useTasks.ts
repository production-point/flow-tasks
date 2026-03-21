import { useQuery, useMutation, useQueryClient } from "@tanstack/react-query";
import { api } from "../lib/api";
import { useSettings } from "./useSettings";
import { enqueue, getQueue, dequeue } from "../lib/offline-queue";
import type { Task, CreateTaskInput, UpdateTaskInput } from "../lib/types";

export function useTasks() {
  const pollInterval = useSettings((s) => s.settings.pollInterval);
  const isConfigured = useSettings((s) => Boolean(s.settings.apiUrl && s.apiKey));

  return useQuery({
    queryKey: ["tasks"],
    queryFn: api.getTasks,
    refetchInterval: pollInterval * 1000,
    enabled: isConfigured,
  });
}

export function useCreateTask() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: (input: CreateTaskInput) => api.createTask(input),
    onError: (_err, input) => {
      enqueue({ type: "create", input });
    },
    onSuccess: () => qc.invalidateQueries({ queryKey: ["tasks"] }),
  });
}

export function useUpdateTask() {
  const qc = useQueryClient();
  return useMutation({
    mutationFn: ({ id, input }: { id: number; input: UpdateTaskInput }) =>
      api.updateTask(id, input),
    onMutate: async ({ id, input }) => {
      await qc.cancelQueries({ queryKey: ["tasks"] });
      const previous = qc.getQueryData<Task[]>(["tasks"]);
      qc.setQueryData<Task[]>(["tasks"], (old) =>
        old?.map((t) => (t.id === id ? { ...t, ...input } : t))
      );
      return { previous };
    },
    onError: (_err, { id, input }, context) => {
      if (context?.previous) qc.setQueryData(["tasks"], context.previous);
      enqueue({ type: "update", taskId: id, input });
    },
    onSettled: () => qc.invalidateQueries({ queryKey: ["tasks"] }),
  });
}

export function useCompleteTask() {
  const update = useUpdateTask();
  return {
    ...update,
    mutate: (id: number) => update.mutate({ id, input: { completed: true } }),
  };
}

export async function replayOfflineQueue(
  onError: (msg: string) => void
): Promise<void> {
  const queue = getQueue();
  for (const mutation of queue) {
    try {
      if (mutation.type === "create") {
        await api.createTask(mutation.input as CreateTaskInput);
      } else if (mutation.type === "update" && mutation.taskId) {
        await api.updateTask(mutation.taskId, mutation.input as UpdateTaskInput);
      }
      dequeue(mutation.id);
    } catch (err) {
      onError(`Failed to sync: ${(err as Error).message}`);
      dequeue(mutation.id);
    }
  }
}
