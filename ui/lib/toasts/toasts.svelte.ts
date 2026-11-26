// Toasts: errors and one-off notices, shown together at the bottom right of the window.
// Errors stay until dismissed; notices go after a few seconds (not while the pointer or focus
// is on the stack). The same message again bumps a count on the one showing.

export type ToastKind = "error" | "notice";

export type Toast = {
  id: number;
  kind: ToastKind;
  text: string;
  /** How many times this message came in while it was showing. */
  count: number;
};

/** How long a notice stays, from when it (or its latest repeat) arrived: this, or longer if
 *  it takes longer to read ([`readingTime`]). */
export const NOTICE_MS = 4000;
/** Time to read `text`: `NOTICE_MS` covers 40 characters, and a longer notice gets a tenth of
 *  a second more for each one over. The summary of an import ("Import stopped · 40 left out ·
 *  212 imported · 3 already in library") is shown nowhere else. */
export const readingTime = (text: string) => Math.max(NOTICE_MS, text.length * 100);
/** At most this many show at once. */
export const MAX_TOASTS = 4;

type Timer = { handle: ReturnType<typeof setTimeout> | null; left: number; since: number };

export class Toasts {
  /** Oldest first. */
  list = $state<Toast[]>([]);
  #next = 1;
  #timers = new Map<number, Timer>();
  #held = false;
  #log: ((text: string) => void) | undefined;

  /** `log` is told about every error, so it can go to the log file too. */
  constructor(opts: { log?: (text: string) => void } = {}) {
    this.#log = opts.log;
  }

  /** Show an error until it's dismissed. `e` may be anything thrown; `context` says what failed,
   *  as in "Couldn't open DSC_0042.NEF". */
  error(context: string, e?: unknown): number {
    const text = e === undefined ? context : `${context}: ${describe(e)}`;
    this.#log?.(text);
    return this.add("error", text);
  }

  /** Show a notice for a few seconds. */
  notice(text: string): number {
    return this.add("notice", text);
  }

  private add(kind: ToastKind, text: string): number {
    const same = this.list.find((t) => t.kind === kind && t.text === text);
    if (same) {
      same.count++;
      if (kind === "notice") this.#arm(same.id, NOTICE_MS);
      return same.id;
    }
    const id = this.#next++;
    this.list.push({ id, kind, text, count: 1 });
    if (kind === "notice") this.#arm(id, NOTICE_MS);
    // Over the limit: the oldest notice makes room, else the oldest error.
    while (this.list.length > MAX_TOASTS) {
      const out = this.list.find((t) => t.kind === "notice") ?? this.list[0];
      this.dismiss(out.id);
    }
    return id;
  }

  dismiss(id: number) {
    this.#disarm(id);
    this.list = this.list.filter((t) => t.id !== id);
  }

  /** Dismiss the one that arrived last. */
  dismissNewest() {
    const last = this.list.at(-1);
    if (last) this.dismiss(last.id);
  }

  clear() {
    for (const id of this.#timers.keys()) this.#disarm(id);
    this.list = [];
  }

  /** The pointer (or focus) is on the stack: keep notices where they are. */
  hold() {
    if (this.#held) return;
    this.#held = true;
    const now = Date.now();
    for (const t of this.#timers.values()) {
      if (t.handle === null) continue;
      clearTimeout(t.handle);
      t.handle = null;
      t.left = Math.max(0, t.left - (now - t.since));
    }
  }

  /** Let notices go again, each with the time it had left. */
  release() {
    if (!this.#held) return;
    this.#held = false;
    // (A copy: arming replaces the map's entries.)
    for (const [id, t] of [...this.#timers]) this.#arm(id, t.left);
  }

  #arm(id: number, ms: number) {
    // A notice starting its time afresh (not one resumed with what it had left).
    if (ms === NOTICE_MS) ms = readingTime(this.list.find((t) => t.id === id)?.text ?? "");
    this.#disarm(id);
    const t: Timer = { handle: null, left: ms, since: Date.now() };
    if (!this.#held) t.handle = setTimeout(() => this.dismiss(id), ms);
    this.#timers.set(id, t);
  }

  #disarm(id: number) {
    const t = this.#timers.get(id);
    if (t?.handle) clearTimeout(t.handle);
    this.#timers.delete(id);
  }
}

/** What went wrong, as text: Tauri commands reject with plain strings, JS with `Error`s. */
export function describe(e: unknown): string {
  if (e instanceof Error) return e.message;
  if (typeof e === "string") return e;
  try {
    return JSON.stringify(e);
  } catch {
    return String(e);
  }
}

const log = (text: string) =>
  import("@tauri-apps/api/core")
    .then(({ invoke }) => invoke("ui_log", { level: "warn", message: text }))
    .catch(() => {});

export const toasts = new Toasts({ log });
