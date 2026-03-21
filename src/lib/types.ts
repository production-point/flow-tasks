export interface Task {
  id: number;
  title: string;
  description: string | null;
  priority: "p1" | "p2" | "p3" | "p4" | null;
  dueDate: string | null;
  dueTime: string | null;
  completed: boolean;
  parentTaskId: number | null;
  assigneeId: number | null;
  sectionId: number | null;
  projectId: number | null;
  labels: number[] | null;
  isRecurring: boolean | null;
  recurringPattern: string | null;
  recurringInterval: number | null;
  createdAt: string;
  updatedAt: string;
}

export interface Project {
  id: number;
  name: string;
  color: string | null;
  status: string | null;
}

export interface Label {
  id: number;
  name: string;
  color: string | null;
}

export interface User {
  id: number;
  firstName: string | null;
  lastName: string | null;
  email: string | null;
  profileImageUrl: string | null;
}

export interface CreateTaskInput {
  title: string;
  description?: string;
  priority?: "p1" | "p2" | "p3" | "p4";
  dueDate?: string;
  dueTime?: string;
  projectId?: number;
  assigneeId?: number;
  labels?: number[];
  parentTaskId?: number;
}

export interface UpdateTaskInput {
  title?: string;
  description?: string;
  priority?: "p1" | "p2" | "p3" | "p4" | null;
  dueDate?: string | null;
  dueTime?: string | null;
  completed?: boolean;
  projectId?: number | null;
  assigneeId?: number | null;
  labels?: number[] | null;
}

export interface AppSettings {
  apiUrl: string;
  pollInterval: number;
  hotkey: string;
  notificationsEnabled: boolean;
  reminderTime: string;
  startOnLogin: boolean;
  windowX: number | null;
  windowY: number | null;
  windowWidth: number;
  windowHeight: number;
  activeTab: "all" | "by-project" | "overdue";
  isPinned: boolean;
}
