/** Parses a camera-local `YYYY-MM-DDTHH:MM:SS` without shifting it through UTC. */
export function parseLocal(stamp: string): Date {
  const [y, mo, d, h = 0, mi = 0, s = 0] = stamp.split(/[-T:]/).map(Number);
  return new Date(y, mo - 1, d, h, mi, s);
}

const monthDay = new Intl.DateTimeFormat(undefined, { month: "long", day: "numeric" });
const weekday = new Intl.DateTimeFormat(undefined, { weekday: "long" });
const dateTime = new Intl.DateTimeFormat(undefined, { dateStyle: "long", timeStyle: "short" });
const timeOnly = new Intl.DateTimeFormat(undefined, { timeStyle: "short" });
const whole = new Intl.NumberFormat();

export const count = (n: number) => whole.format(n);

export const plural = (n: number, one: string, many = `${one}s`) => `${count(n)} ${n === 1 ? one : many}`;

/** "September 19" plus "Saturday, 2026" for a day heading. */
export function dayHeading(day: string): { title: string; detail: string } {
  const date = parseLocal(day);
  return { title: monthDay.format(date), detail: `${weekday.format(date)}, ${date.getFullYear()}` };
}

export const longDateTime = (date: Date) => dateTime.format(date);
export const shortTime = (date: Date) => timeOnly.format(date);

export function shutter(seconds: number): string {
  if (seconds >= 0.4) return `${+seconds.toFixed(1)}s`;
  return `1/${Math.round(1 / seconds)}s`;
}

export const aperture = (f: number) => `ƒ/${+f.toFixed(1)}`;
export const focalLength = (mm: number) => `${Math.round(mm)} mm`;

export function fileSize(bytes: number): string {
  if (bytes >= 1e9) return `${(bytes / 1e9).toFixed(2)} GB`;
  if (bytes >= 1e6) return `${(bytes / 1e6).toFixed(1)} MB`;
  return `${Math.max(1, Math.round(bytes / 1e3))} KB`;
}

export function megapixels(width: number, height: number): string {
  return `${((width * height) / 1e6).toFixed(1)} MP`;
}

const shortDate = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

/** How long ago a moment (Unix seconds) was, briefly: "now", "5 min", "3 h", "Sep 19". */
export function ago(moment: number): string {
  const seconds = Date.now() / 1000 - moment;
  if (seconds < 60) return "now";
  if (seconds < 3600) return `${Math.floor(seconds / 60)} min`;
  if (seconds < 86400) return `${Math.floor(seconds / 3600)} h`;
  return shortDate.format(new Date(moment * 1000));
}

/** Whole days until a photo deleted at `deletedAt` (Unix seconds) is removed for good. */
export function daysLeft(deletedAt: number, retentionDays = 30): number {
  const elapsed = Date.now() / 1000 - deletedAt;
  return Math.max(0, Math.ceil(retentionDays - elapsed / 86400));
}
