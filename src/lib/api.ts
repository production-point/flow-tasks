import { useSettings } from "../hooks/useSettings";
import type { Task, Project, Label, User, CreateTaskInput, UpdateTaskInput } from "./types";

class ApiClient {
  private get baseUrl(): string {
    return useSettings.getState().settings.apiUrl.replace(/\/$/, "");
  }

  private get headers(): HeadersInit {
    const apiKey = useSettings.getState().apiKey;
    return {
      "Content-Type": "application/json",
      ...(apiKey ? { Authorization: `Bearer ${apiKey}` } : {}),
    };
  }

  private async request<T>(path: string, options?: RequestInit): Promise<T> {
    const res = await fetch(`${this.baseUrl}${path}`, {
      ...options,
      headers: { ...this.headers, ...options?.headers },
    });
    if (!res.ok) {
      throw new Error(`API ${res.status}: ${await res.text()}`);
    }
    return res.json();
  }

  getTasks = () => this.request<Task[]>("/api/v1/tasks");
  getTask = (id: number) => this.request<Task>(`/api/v1/tasks/${id}`);
  createTask = (input: CreateTaskInput) =>
    this.request<Task>("/api/v1/tasks", { method: "POST", body: JSON.stringify(input) });
  updateTask = (id: number, input: UpdateTaskInput) =>
    this.request<Task>(`/api/v1/tasks/${id}`, { method: "PATCH", body: JSON.stringify(input) });
  getProjects = () => this.request<Project[]>("/api/v1/projects");
  getLabels = () => this.request<Label[]>("/api/v1/labels");
  getUsers = () => this.request<User[]>("/api/v1/users");
}

export const api = new ApiClient();
