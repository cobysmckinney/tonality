// Capture-day grouping for the Library grid.

/** `YYYY-MM-DD` of an ISO 8601 capture time, or "" when the photo has none. */
export function dayKey(capturedAt: string | null | undefined): string {
  return capturedAt && /^\d{4}-\d{2}-\d{2}/.test(capturedAt) ? capturedAt.slice(0, 10) : "";
}

/** "Today", "Yesterday", "Saturday, November 2" (this year) or "November 2, 2025". */
export function dayLabel(key: string, now = new Date()): string {
  if (!key) return "No date";
  const [y, m, d] = key.split("-").map(Number);
  const date = new Date(y, m - 1, d);
  const today = new Date(now.getFullYear(), now.getMonth(), now.getDate());
  const days = Math.round((today.getTime() - date.getTime()) / 86_400_000);
  if (days === 0) return "Today";
  if (days === 1) return "Yesterday";
  if (y === now.getFullYear()) {
    return date.toLocaleDateString(undefined, { weekday: "long", month: "long", day: "numeric" });
  }
  return date.toLocaleDateString(undefined, { month: "long", day: "numeric", year: "numeric" });
}
