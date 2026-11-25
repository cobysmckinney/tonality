// When steps were made, shown only where it says something: steps are grouped into editing
// sessions, and each session gets one time label.

/** A pause longer than this between two steps starts a new session. */
export const SESSION_GAP_MS = 30 * 60 * 1000;

/**
 * Where each session starts, for steps listed newest first: the id of the newest step of each
 * session, mapped to the time the session began (its oldest step).
 */
export function sessionLabels(
  steps: { id: number; time: number }[],
  gap = SESSION_GAP_MS,
): Map<number, number> {
  const out = new Map<number, number>();
  let first: number | null = null; // newest step of the session being walked
  let start = 0;
  let prev: number | null = null;
  for (const s of steps) {
    if (first === null || prev === null || Math.abs(prev - s.time) > gap) {
      if (first !== null) out.set(first, start);
      first = s.id;
      start = s.time;
    }
    start = Math.min(start, s.time);
    prev = s.time;
  }
  if (first !== null) out.set(first, start);
  return out;
}

const startOfDay = (ms: number) => {
  const d = new Date(ms);
  d.setHours(0, 0, 0, 0);
  return d.getTime();
};

const clock = (ms: number) => {
  const d = new Date(ms);
  return `${String(d.getHours()).padStart(2, "0")}:${String(d.getMinutes()).padStart(2, "0")}`;
};

/** A session's label: "Today 14:02", "Yesterday 09:30", "Nov 12", or "Nov 12, 2025". */
export function sessionLabel(ms: number, now = Date.now()): string {
  const days = Math.round((startOfDay(now) - startOfDay(ms)) / 86_400_000);
  if (days <= 0) return `Today ${clock(ms)}`;
  if (days === 1) return `Yesterday ${clock(ms)}`;
  const sameYear = new Date(ms).getFullYear() === new Date(now).getFullYear();
  return new Date(ms).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    year: sameYear ? undefined : "numeric",
  });
}

/** One step's own time, for its details: "14:05", or "Nov 12, 14:05" on another day. */
export function stepTime(ms: number, now = Date.now()): string {
  if (startOfDay(ms) === startOfDay(now)) return clock(ms);
  return `${sessionLabel(ms, now).replace(/ \d\d:\d\d$/, "")}, ${clock(ms)}`;
}
