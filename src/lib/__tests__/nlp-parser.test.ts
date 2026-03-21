import { describe, it, expect } from "vitest";
import { parseTaskInput } from "../nlp-parser";

describe("parseTaskInput", () => {
  it("extracts priority", () => {
    const result = parseTaskInput("Review contracts p1", []);
    expect(result.title).toBe("Review contracts");
    expect(result.priority).toBe("p1");
  });

  it("extracts due date - tomorrow", () => {
    const result = parseTaskInput("Send invoice tomorrow", []);
    expect(result.title).toBe("Send invoice");
    expect(result.dueDate).toBeDefined();
  });

  it("extracts due date - day name", () => {
    const result = parseTaskInput("Review deck by friday", []);
    expect(result.title).toBe("Review deck");
    expect(result.dueDate).toBeDefined();
  });

  it("extracts due date - today", () => {
    const result = parseTaskInput("Call supplier today", []);
    expect(result.title).toBe("Call supplier");
    expect(result.dueDate).toBeDefined();
  });

  it("fuzzy matches project name", () => {
    const projects = [
      { id: 1, name: "Glastonbury 2026", color: null, status: null },
      { id: 2, name: "BST Hyde Park", color: null, status: null },
    ];
    const result = parseTaskInput("Book crew for glastonbury p2", projects);
    expect(result.title).toBe("Book crew");
    expect(result.projectId).toBe(1);
    expect(result.priority).toBe("p2");
  });

  it("handles plain text with no markers", () => {
    const result = parseTaskInput("Just a simple task", []);
    expect(result.title).toBe("Just a simple task");
    expect(result.priority).toBeUndefined();
    expect(result.dueDate).toBeUndefined();
    expect(result.projectId).toBeUndefined();
  });

  it("handles 'for project' syntax", () => {
    const projects = [{ id: 5, name: "BST Hyde Park", color: null, status: null }];
    const result = parseTaskInput("Book site survey for BST Hyde Park", projects);
    expect(result.projectId).toBe(5);
  });
});
