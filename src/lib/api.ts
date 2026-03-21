import { fetch as tauriFetch } from "@tauri-apps/plugin-http";
import { useSettings } from "../hooks/useSettings";
import type { Task, Project, Label, User, CreateTaskInput, UpdateTaskInput } from "./types";

class ApiClient {
  private get baseUrl(): string {
    return useSettings.getState().settings.apiUrl.replace(/\/$/, "");
  }

  private get headers(): Record<string, string> {
    const apiKey = useSettings.getState().apiKey;
    return {
      "Content-Type": "application/json",
      ...(apiKey ? { Authorization: `Bearer ${apiKey}` } : {}),
    };
  }

  private async request<T>(path: string, options?: RequestInit): Promise<T> {
    const url = `${this.baseUrl}${path}`;
    const headers = { ...this.headers, ...(options?.headers as Record<string, string>) };

    // Use Tauri's HTTP plugin which bypasses webview restrictions
    let res: Response;
    try {
      res = await tauriFetch(url, {
        ...options,
        headers,
      });
    } catch {
      // Fallback to browser fetch (dev mode)
      res = await globalThis.fetch(url, { ...options, headers });
    }

    if (!res.ok) {
      const text = await res.text();
      throw new Error(`API ${res.status}: ${text}`);
    }
    return res.json();
  }

  getTasks = () => this.request<Task[]>("/api/v1/tasks");
  getTask = (id: string) => this.request<Task>(`/api/v1/tasks/${id}`);
  createTask = (input: CreateTaskInput) =>
    this.request<Task>("/api/v1/tasks", { method: "POST", body: JSON.stringify(input) });
  updateTask = (id: string, input: UpdateTaskInput) =>
    this.request<Task>(`/api/v1/tasks/${id}`, { method: "PATCH", body: JSON.stringify(input) });
  getProjects = () => this.request<Project[]>("/api/v1/projects");
  getLabels = () => this.request<Label[]>("/api/v1/labels");
  getUsers = () => this.request<User[]>("/api/v1/users");
}

export const api = new ApiClient();
