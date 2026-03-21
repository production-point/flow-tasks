/**
 * Normalizes any date string (ISO timestamp or YYYY-MM-DD) to YYYY-MM-DD format.
 * Returns null if the input is null/undefined or not a valid date.
 */
export function toDateString(d: string | null | undefined): string | null {
  if (!d) return null;
  try {
    return new Date(d).toISOString().split("T")[0];
  } catch {
    return null;
  }
}
