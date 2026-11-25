// Panel sections (`Section.svelte`): whether each is open is kept per section in localStorage;
// "Open one section at a time" (Settings) makes opening one close the others shown with it.
// A per-viewer preference, kept like the other UI conveniences (filmstrip, open sections).

const ONE_KEY = "tonality.sections.oneAtATime";

function read(key: string): string | null {
  try {
    return localStorage.getItem(key);
  } catch {
    return null; // storage unavailable: defaults
  }
}
function write(key: string, v: string) {
  try {
    localStorage.setItem(key, v);
  } catch {
    /* per-viewer convenience only */
  }
}

/** Whether the section stored under `key` was left open; `fallback` if never toggled. */
export function wasOpen(key: string, fallback: boolean): boolean {
  const v = read(`tonality.section.${key}`);
  return v === null ? fallback : v === "1";
}
export function rememberOpen(key: string, open: boolean) {
  write(`tonality.section.${key}`, open ? "1" : "0");
}

class Sections {
  oneAtATime = $state(read(ONE_KEY) === "1");
  /** The section opened last (its key) and a counter, so each opening is seen. */
  opened = $state<{ key: string; n: number } | null>(null);
  private n = 0;

  setOneAtATime(on: boolean) {
    this.oneAtATime = on;
    write(ONE_KEY, on ? "1" : "0");
  }

  /** A section was opened by the user. */
  open(key: string) {
    this.opened = { key, n: ++this.n };
  }
}

export const sections = new Sections();
