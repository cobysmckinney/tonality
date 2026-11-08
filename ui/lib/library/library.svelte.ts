// Library state: imports, the current photo list, filters and selection.
// Module-level so scroll position and selection survive a trip to Edit and back.
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { open } from "@tauri-apps/plugin-dialog";
import { SvelteSet } from "svelte/reactivity";
import { api, RAW_EXTENSIONS } from "$lib/api";
import { app } from "$lib/app.svelte";
import type { ImageQuery } from "$lib/bindings/ImageQuery";
import type { ImageRow } from "$lib/bindings/ImageRow";
import type { ImportInfo } from "$lib/bindings/ImportInfo";
import { session } from "$lib/edit/session.svelte";

const SIZE_KEY = "tonality.library.thumbSize";
const RAW_FILTER = [{ name: "RAW photos", extensions: [...RAW_EXTENSIONS, ...RAW_EXTENSIONS.map((e) => e.toUpperCase())] }];

function storedSize(): number {
  try {
    const v = Number(localStorage.getItem(SIZE_KEY));
    return v >= 100 && v <= 360 ? v : 180;
  } catch {
    return 180;
  }
}

class LibraryState {
  imports = $state<ImportInfo[]>([]);
  total = $state(0);
  root = $state("");
  /** Photos matching the current query. Raw (not deeply reactive): replaced wholesale. */
  images = $state.raw<ImageRow[]>([]);
  query = $state<ImageQuery>({ import: null, minRating: 0, flag: null, sort: "captureTime", descending: false });
  selected = new SvelteSet<number>();
  /** Keyboard focus / range anchor (image id). */
  focus = $state<number | null>(null);
  thumbSize = $state(storedSize());
  importing = $state<{ done: number; total: number } | null>(null);
  /** Files are being dragged over the window. */
  dropping = $state(false);
  message = $state<string | null>(null);
  loaded = $state(false);
  scrollTop = 0;

  constructor() {
    listen<{ done: number; total: number }>("import-progress", (e) => {
      if (this.importing) this.importing = e.payload;
    });
    getCurrentWebview()
      .onDragDropEvent((e) => {
        const p = e.payload;
        if (p.type === "enter" || p.type === "over") this.dropping = true;
        else if (p.type === "leave") this.dropping = false;
        else if (p.type === "drop") {
          this.dropping = false;
          if (p.paths.length) this.importPaths(p.paths, { open: app.mode === "edit" });
        }
      })
      .catch(() => {});
  }

  get focused(): ImageRow | null {
    return this.images.find((i) => i.id === this.focus) ?? null;
  }

  setThumbSize(v: number) {
    this.thumbSize = v;
    try {
      localStorage.setItem(SIZE_KEY, String(v));
    } catch {
      /* per-viewer convenience only */
    }
  }

  async refresh() {
    try {
      const [overview, images] = await Promise.all([api.library.overview(), api.library.images(this.query)]);
      this.imports = overview.imports;
      this.total = overview.total;
      this.root = overview.root;
      this.images = images;
      // Drop selection of photos that are no longer listed.
      const ids = new Set(images.map((i) => i.id));
      for (const id of this.selected) if (!ids.has(id)) this.selected.delete(id);
      if (this.focus !== null && !ids.has(this.focus)) this.focus = null;
    } catch (e) {
      this.message = String(e);
    } finally {
      this.loaded = true;
    }
  }

  setQuery(patch: Partial<ImageQuery>) {
    this.query = { ...this.query, ...patch };
    this.scrollTop = 0;
    this.refresh();
  }

  async importFolder() {
    const path = await open({ directory: true, multiple: false, title: "Import a folder" });
    if (typeof path === "string") await this.importPaths([path]);
  }

  async importFiles(opts: { open?: boolean } = {}) {
    const paths = await open({ multiple: true, directory: false, title: "Import photos", filters: RAW_FILTER });
    if (paths?.length) await this.importPaths(paths, opts);
  }

  /** Copy files/folders into the library. With `open`, the first photo opens in Edit. */
  async importPaths(paths: string[], opts: { open?: boolean } = {}) {
    if (this.importing) return;
    this.importing = { done: 0, total: 0 };
    this.message = null;
    try {
      const s = await api.library.import(paths);
      const parts: string[] = [];
      if (s.added) parts.push(`${s.added} imported`);
      if (s.duplicates) parts.push(`${s.duplicates} already in library`);
      if (s.failed) parts.push(`${s.failed} failed`);
      this.message = parts.length ? parts.join(" · ") : (s.error ?? "Nothing to import");
      if (s.failed && s.error) console.warn("[import]", s.error);
      // Show what just arrived.
      this.query = { ...this.query, import: s.import?.id ?? this.query.import };
      this.scrollTop = 0;
      await this.refresh();
      if (s.images.length) {
        this.selected.clear();
        this.focus = s.images[0];
        this.selected.add(s.images[0]);
        if (opts.open) this.open(s.images[0]);
      }
    } catch (e) {
      this.message = String(e);
    } finally {
      this.importing = null;
    }
  }

  // ---- selection ----

  select(id: number, e?: { shiftKey?: boolean; ctrlKey?: boolean; metaKey?: boolean }) {
    if (e?.shiftKey && this.focus !== null) {
      const a = this.images.findIndex((i) => i.id === this.focus);
      const b = this.images.findIndex((i) => i.id === id);
      if (a >= 0 && b >= 0) {
        if (!(e.ctrlKey || e.metaKey)) this.selected.clear();
        for (let i = Math.min(a, b); i <= Math.max(a, b); i++) this.selected.add(this.images[i].id);
        return; // keep the anchor
      }
    }
    if (e?.ctrlKey || e?.metaKey) {
      if (this.selected.has(id)) this.selected.delete(id);
      else this.selected.add(id);
    } else {
      this.selected.clear();
      this.selected.add(id);
    }
    this.focus = id;
  }

  selectAll() {
    for (const i of this.images) this.selected.add(i.id);
  }

  clearSelection() {
    this.selected.clear();
  }

  /** Move focus by `delta` positions (±1 horizontally, ±cols vertically). */
  move(delta: number, extend: boolean) {
    if (!this.images.length) return;
    const cur = this.images.findIndex((i) => i.id === this.focus);
    const next = cur < 0 ? 0 : Math.max(0, Math.min(this.images.length - 1, cur + delta));
    const id = this.images[next].id;
    if (extend) {
      this.selected.add(id);
      this.focus = id;
    } else this.select(id);
  }

  /** Photos an action applies to: the selection, else the focused photo. */
  private targets(): number[] {
    if (this.selected.size) return [...this.selected];
    return this.focus !== null ? [this.focus] : [];
  }

  private patch(ids: number[], f: (i: ImageRow) => ImageRow) {
    const set = new Set(ids);
    this.images = this.images.map((i) => (set.has(i.id) ? f(i) : i));
  }

  async setRating(rating: number) {
    const ids = this.targets();
    if (!ids.length) return;
    // Pressing the current rating again clears it.
    const all = ids.every((id) => this.images.find((i) => i.id === id)?.rating === rating);
    const r = all ? 0 : rating;
    this.patch(ids, (i) => ({ ...i, rating: r }));
    await api.library.setRating(ids, r).catch((e) => (this.message = String(e)));
  }

  async setFlag(flag: number) {
    const ids = this.targets();
    if (!ids.length) return;
    this.patch(ids, (i) => ({ ...i, flag }));
    await api.library.setFlag(ids, flag).catch((e) => (this.message = String(e)));
  }

  /** Open a photo (default: the focused one) in Edit. */
  open(id = this.focus) {
    if (id === null) return;
    this.focus = id;
    app.mode = "edit";
    if (session.image?.id !== id) session.open(id);
  }
}

export const library = new LibraryState();
