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
const MAX_CACHED = 1500;

// Keyed by `id:version`: a thumbnail changes when the photo's shown version does.
const cache = new Map<string, string>(); // key → blob URL, insertion-ordered for LRU
const failed = new Set<string>();
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
  for (const k of [...failed]) if (k.startsWith(`${id}:`)) failed.delete(k);
  epochs.set(id, (epochs.get(id) ?? 0) + 1);
}
const queue: Waiter[] = [];
let inFlight = 0;

function remember(id: string, url: string) {
  cache.set(id, url);
  if (cache.size > MAX_CACHED) {
    const [oldest, old] = cache.entries().next().value!;
    cache.delete(oldest);
    URL.revokeObjectURL(old);
  }
}

function pump() {
  while (inFlight < MAX_IN_FLIGHT && queue.length) {
    const w = queue.pop()!; // LIFO: newest requests are what's on screen
    if (w.cancelled) continue;
    const hit = cache.get(w.key);
    if (hit) {
      w.resolve(hit);
      continue;
    }
    inFlight++;
    fetch(`${thumbUrl(w.id)}?v=${encodeURIComponent(w.key)}`)
      .then(async (r) => {
        if (!r.ok) throw new Error(await r.text());
        const url = URL.createObjectURL(await r.blob());
        remember(w.key, url);
        w.resolve(url);
      })
      .catch((e) => {
        console.warn(`[thumbs] ${w.id}: ${e}`);
        failed.add(w.key);
        w.resolve(null);
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

  function load(key: string) {
    const id = Number(key.split(":")[0]);
    if (waiter) waiter.cancelled = true;
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
    if (failed.has(key)) {
      img.classList.add("failed");
      return;
    }
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
    },
  };
}
