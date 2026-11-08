// Thumbnail loader.
//
// Thumbnails come from the `thumb://` protocol, but they're fetched and handed to <img> as blob
// URLs rather than used directly: WebKitGTK won't load custom-scheme images into a page served
// from http:// (the Vite dev server), and doing it ourselves gives us a priority queue anyway:
// the most recently requested (i.e. currently visible) cells load first, and cells that scroll
// away before their turn are dropped without ever hitting the backend.
import { thumbUrl } from "$lib/api";

const MAX_IN_FLIGHT = 6;
const MAX_CACHED = 1500;

const cache = new Map<number, string>(); // id → blob URL, insertion-ordered for LRU
const failed = new Set<number>();
type Waiter = { id: number; resolve: (url: string | null) => void; cancelled: boolean };
const queue: Waiter[] = [];
let inFlight = 0;

function remember(id: number, url: string) {
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
    const hit = cache.get(w.id);
    if (hit) {
      w.resolve(hit);
      continue;
    }
    inFlight++;
    fetch(thumbUrl(w.id))
      .then(async (r) => {
        if (!r.ok) throw new Error(await r.text());
        const url = URL.createObjectURL(await r.blob());
        remember(w.id, url);
        w.resolve(url);
      })
      .catch((e) => {
        console.warn(`[thumbs] ${w.id}: ${e}`);
        failed.add(w.id);
        w.resolve(null);
      })
      .finally(() => {
        inFlight--;
        pump();
      });
  }
}

/** Svelte action: `<img use:thumbnail={id}>`. */
export function thumbnail(img: HTMLImageElement, id: number) {
  let waiter: Waiter | null = null;

  function load(id: number) {
    if (waiter) waiter.cancelled = true;
    img.removeAttribute("src");
    img.classList.remove("loaded", "failed");
    const hit = cache.get(id);
    if (hit) {
      // Refresh LRU position.
      cache.delete(id);
      cache.set(id, hit);
      img.src = hit;
      img.classList.add("loaded");
      return;
    }
    if (failed.has(id)) {
      img.classList.add("failed");
      return;
    }
    const w: Waiter = {
      id,
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

  load(id);
  return {
    update: load,
    destroy() {
      if (waiter) waiter.cancelled = true;
    },
  };
}
