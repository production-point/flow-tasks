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

    // In the packaged app the Tauri HTTP plugin is the only viable transport —
    // it bypasses the webview's CORS. We only fall back to the browser's fetch
    // when running outside Tauri (e.g. `npm run dev` in a browser). Falling back
    // inside the app just hits CORS and masks the real plugin error with a
    // generic "Load failed", so surface the plugin error instead.
    const inTauri = typeof window !== "undefined" && "__TAURI_INTERNALS__" in window;
    let res: Response;
    try {
      res = await tauriFetch(url, {
        ...options,
        headers,
      });
    } catch (tauriErr) {
      if (inTauri) {
        const detail = tauriErr instanceof Error ? tauriErr.message : String(tauriErr);
        throw new Error(`HTTP request failed: ${detail}`);
      }
      // Dev browser only.
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
