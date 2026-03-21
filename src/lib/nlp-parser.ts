import type { CreateTaskInput, Project } from "./types";

const PRIORITY_REGEX = /\b(p[1-4])\b/i;
const TODAY_REGEX = /\btoday\b/i;
const TOMORROW_REGEX = /\btomorrow\b/i;
const DAY_REGEX = /\b(?:by\s+)?(monday|tuesday|wednesday|thursday|friday|saturday|sunday)\b/i;

const DAYS = ["sunday", "monday", "tuesday", "wednesday", "thursday", "friday", "saturday"];

function getNextDayDate(dayName: string): string {
  const target = DAYS.indexOf(dayName.toLowerCase());
  const today = new Date();
  let diff = target - today.getDay();
  if (diff <= 0) diff += 7;
  const date = new Date(today);
  date.setDate(today.getDate() + diff);
  return date.toISOString().split("T")[0];
}

function fuzzyMatchProject(text: string, projects: Project[]): Project | null {
  const lower = text.toLowerCase();
  for (const p of projects) {
    if (lower.includes(p.name.toLowerCase())) return p;
  }
  for (const p of projects) {
    const firstName = p.name.split(/\s+/)[0].toLowerCase();
    if (firstName.length > 3 && lower.includes(firstName)) return p;
  }
  return null;
}

function escapeRegex(str: string): string {
  return str.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
}

export function parseTaskInput(input: string, projects: Project[]): CreateTaskInput {
  let text = input.trim();
  let priority: CreateTaskInput["priority"];
  let dueDate: string | undefined;
  let projectId: number | undefined;

  // Extract priority
  const pMatch = text.match(PRIORITY_REGEX);
  if (pMatch) {
    priority = pMatch[1].toLowerCase() as CreateTaskInput["priority"];
    text = text.replace(PRIORITY_REGEX, "").trim();
  }

  // Extract due date
  if (TODAY_REGEX.test(text)) {
    dueDate = new Date().toISOString().split("T")[0];
    text = text.replace(TODAY_REGEX, "").trim();
  } else if (TOMORROW_REGEX.test(text)) {
    const d = new Date();
    d.setDate(d.getDate() + 1);
    dueDate = d.toISOString().split("T")[0];
    text = text.replace(TOMORROW_REGEX, "").trim();
  } else {
    const dayMatch = text.match(DAY_REGEX);
    if (dayMatch) {
      dueDate = getNextDayDate(dayMatch[1]);
      text = text.replace(DAY_REGEX, "").trim();
    }
  }

  // Extract project
  if (projects.length > 0) {
    // Try "for <project>" pattern
    const forMatch = text.match(/\bfor\s+(.+?)$/i);
    if (forMatch) {
      const matched = fuzzyMatchProject(forMatch[1], projects);
      if (matched) {
        projectId = matched.id;
        // Remove the entire "for ..." tail that was captured
        text = text.slice(0, forMatch.index).trim();
      }
    }
    // Fallback: fuzzy match anywhere
    if (!projectId) {
      const matched = fuzzyMatchProject(text, projects);
      if (matched) {
        projectId = matched.id;
        // Try exact name first, then first word
        const nameRegex = new RegExp(escapeRegex(matched.name), "i");
        if (nameRegex.test(text)) {
          text = text.replace(nameRegex, "").trim();
        } else {
          const firstName = matched.name.split(/\s+/)[0];
          if (firstName.length > 3) {
            const firstNameRegex = new RegExp(escapeRegex(firstName), "i");
            text = text.replace(firstNameRegex, "").trim();
          }
        }
      }
    }
  }

  // Clean up
  text = text.replace(/\s+/g, " ").replace(/^[\s,\-\u2013]+|[\s,\-\u2013]+$/g, "").trim();

  return {
    title: text || input.trim(),
    ...(priority && { priority }),
    ...(dueDate && { dueDate }),
    ...(projectId && { projectId }),
  };
}
