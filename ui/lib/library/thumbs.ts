// Thumbnail loader.
//
// Thumbnails come from the `thumb://` protocol, but they're fetched and handed to <img> as blob
// URLs rather than used directly: WebKitGTK won't load custom-scheme images into a page served
// from http:// (the Vite dev server), and doing it ourselves gives us a priority queue anyway:
// the most recently requested (i.e. currently visible) cells load first, and cells that scroll
// away before their turn are dropped without ever hitting the backend.
import { SvelteMap } from "svelte/reactivity";
import { thumbUrl } from "$lib/api";

const MAX_IN_FLIGHT = 6;
/** Blob URLs kept (640 px JPEGs, ~60–100 kB each). Several screens' worth of grid. */
const MAX_CACHED = 600;
/** A thumbnail that failed (e.g. still importing) is tried again after this long. */
const RETRY_MS = 30_000;

// Keyed by `id:version`: a thumbnail changes when the photo's shown version does.
const cache = new Map<string, string>(); // key → blob URL, insertion-ordered for LRU
const failed = new Map<string, number>(); // key → when it failed
/** Keys some <img> is showing right now: never evicted (revoking would blank them). */
const inUse = new Map<string, number>();
type Waiter = { id: number; key: string; resolve: (url: string | null) => void; cancelled: boolean };

/** Bumped when Edit saves a new rendering of a photo, so grid cells reload it. */
const epochs = new SvelteMap<number, number>();

/** The thumbnail key for a photo whose HEAD commit is `head` (reactive). */
export function thumbKey(id: number, head: number | null): string {
  return `${id}:${head ?? 0}:${epochs.get(id) ?? 0}`;
}

/** Forget a photo's cached thumbnail (its edited rendering changed). */
export function invalidateThumb(id: number) {
  for (const k of [...cache.keys()]) {
    if (k.startsWith(`${id}:`)) {
      URL.revokeObjectURL(cache.get(k)!);
      cache.delete(k);
    }
  }
  for (const k of [...failed.keys()]) if (k.startsWith(`${id}:`)) failed.delete(k);
  epochs.set(id, (epochs.get(id) ?? 0) + 1);
}
const queue: Waiter[] = [];
let inFlight = 0;

function remember(id: string, url: string) {
  cache.set(id, url);
  if (cache.size <= MAX_CACHED) return;
  for (const [k, old] of cache) {
    if (inUse.has(k)) continue;
    cache.delete(k);
    URL.revokeObjectURL(old);
    if (cache.size <= MAX_CACHED) break;
  }
}

function use(key: string, by: number) {
  const n = (inUse.get(key) ?? 0) + by;
  if (n > 0) inUse.set(key, n);
  else inUse.delete(key);
}

/** Fetches under way, by key: a second request for the same thumbnail waits for the first. */
const fetching = new Map<string, Waiter[]>();

function pump() {
  while (inFlight < MAX_IN_FLIGHT && queue.length) {
    const w = queue.pop()!; // LIFO: newest requests are what's on screen
    if (w.cancelled) continue;
    const hit = cache.get(w.key);
    if (hit) {
      w.resolve(hit);
      continue;
    }
    const joined = fetching.get(w.key);
    if (joined) {
      joined.push(w);
      continue;
    }
    const waiters = [w];
    fetching.set(w.key, waiters);
    inFlight++;
    const done = (url: string | null) => {
      fetching.delete(w.key);
      for (const x of waiters) if (!x.cancelled) x.resolve(url);
    };
    fetch(`${thumbUrl(w.id)}?v=${encodeURIComponent(w.key)}`)
      .then(async (r) => {
        if (!r.ok) throw new Error(await r.text());
        const url = URL.createObjectURL(await r.blob());
        remember(w.key, url);
        done(url);
      })
      .catch((e) => {
        console.warn(`[thumbs] ${w.id}: ${e}`);
        failed.set(w.key, Date.now());
        done(null);
      })
      .finally(() => {
        inFlight--;
        pump();
      });
  }
}

/** Svelte action: `<img use:thumbnail={thumbKey(id, head)}>`. */
export function thumbnail(img: HTMLImageElement, key: string) {
  let waiter: Waiter | null = null;
  let shown: string | null = null;
  const show = (k: string | null) => {
    if (shown) use(shown, -1);
    shown = k;
    if (k) use(k, 1);
  };

  function load(key: string) {
    const id = Number(key.split(":")[0]);
    if (waiter) waiter.cancelled = true;
    show(key);
    img.removeAttribute("src");
    img.classList.remove("loaded", "failed");
    const hit = cache.get(key);
    if (hit) {
      // Refresh LRU position.
      cache.delete(key);
      cache.set(key, hit);
      img.src = hit;
      img.classList.add("loaded");
      return;
    }
    const at = failed.get(key);
    if (at !== undefined && Date.now() - at < RETRY_MS) {
      img.classList.add("failed");
      return;
    }
    failed.delete(key);
    const w: Waiter = {
      id,
      key,
      cancelled: false,
      resolve: (url) => {
        if (w.cancelled) return;
        if (url) {
          img.src = url;
          img.classList.add("loaded");
        } else img.classList.add("failed");
      },
    };
    waiter = w;
    queue.push(w);
    pump();
  }

  load(key);
  return {
    update: load,
    destroy() {
      if (waiter) waiter.cancelled = true;
      show(null);
    },
  };
}
