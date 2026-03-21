export interface Task {
  id: string;
  title: string;
  description: string | null;
  priority: "p1" | "p2" | "p3" | "p4" | null;
  dueDate: string | null;
  dueTime: string | null;
  completed: boolean;
  parentTaskId: string | null;
  assigneeId: string | null;
  sectionId: string | null;
  projectId: string | null;
  labels: string[] | null;
  isRecurring: boolean | null;
  recurringPattern: string | null;
  recurringInterval: number | null;
  createdAt: string;
  updatedAt: string;
}

export interface Project {
  id: string;
  name: string;
  color: string | null;
  status: string | null;
}

export interface Label {
  id: string;
  name: string;
  color: string | null;
}

export interface User {
  id: string;
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
  projectId?: string;
  assigneeId?: string;
  labels?: string[];
  parentTaskId?: string;
}

export interface UpdateTaskInput {
  title?: string;
  description?: string;
  priority?: "p1" | "p2" | "p3" | "p4" | null;
  dueDate?: string | null;
  dueTime?: string | null;
  completed?: boolean;
  projectId?: string | null;
  assigneeId?: string | null;
  labels?: string[] | null;
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
