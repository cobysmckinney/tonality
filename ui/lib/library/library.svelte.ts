// Library state: imports, the current photo list, filters and selection.
// Module-level so scroll position and selection survive a trip to Edit and back.
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { SvelteSet } from "svelte/reactivity";
import { api, RAW_EXTENSIONS } from "$lib/api";
import { app } from "$lib/app.svelte";
import type { AlbumInfo } from "$lib/bindings/AlbumInfo";
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

/** What the grid is showing. */
export type View = { kind: "all" } | { kind: "album"; id: number } | { kind: "import"; id: number };

/** Photos being dragged from the grid (onto an album). */
export interface PhotoDrag {
  ids: number[];
  x: number;
  y: number;
  /** Album under the pointer. */
  over: number | null;
}

class LibraryState {
  imports = $state<ImportInfo[]>([]);
  albums = $state<AlbumInfo[]>([]);
  drag = $state<PhotoDrag | null>(null);
  total = $state(0);
  root = $state("");
  /** Photos matching the current query. Raw (not deeply reactive): replaced wholesale. */
  images = $state.raw<ImageRow[]>([]);
  query = $state<ImageQuery>({ import: null, album: null, minRating: 0, flag: null, sort: "captureTime", descending: true });
  selected = new SvelteSet<number>();
  /** Keyboard focus / range anchor (image id). */
  focus = $state<number | null>(null);
  thumbSize = $state(storedSize());
  importing = $state<{ done: number; total: number } | null>(null);
  /** The launch-time scan for untracked originals (shown only once it finds some). */
  restoring = $state<{ done: number; total: number } | null>(null);
  /** Files are being dragged over the window. */
  dropping = $state(false);
  message = $state<string | null>(null);
  loaded = $state(false);
  scrollTop = 0;

  constructor() {
    listen<{ done: number; total: number }>("import-progress", (e) => {
      if (this.importing) this.importing = e.payload;
      else if (this.restoring) this.restoring = e.payload;
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

  get view(): View {
    if (this.query.album !== null) return { kind: "album", id: this.query.album };
    if (this.query.import !== null) return { kind: "import", id: this.query.import };
    return { kind: "all" };
  }

  get viewName(): string {
    const v = this.view;
    if (v.kind === "album") return this.albums.find((a) => a.id === v.id)?.name ?? "Album";
    if (v.kind === "import") return this.imports.find((m) => m.id === v.id)?.name ?? "Import";
    return "All Photos";
  }

  show(v: View) {
    this.setQuery({ album: v.kind === "album" ? v.id : null, import: v.kind === "import" ? v.id : null });
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

  async refresh(): Promise<void> {
    try {
      const [overview, images] = await Promise.all([api.library.overview(), api.library.images(this.query)]);
      this.imports = overview.imports;
      this.albums = overview.albums;
      // The album being shown was deleted.
      if (this.query.album !== null && !overview.albums.some((a) => a.id === this.query.album)) {
        this.query = { ...this.query, album: null };
        return this.refresh();
      }
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
      if (s.restored) parts.push(`edits restored for ${s.restored}`);
      if (s.failed) parts.push(`${s.failed} failed`);
      this.message = parts.length ? parts.join(" · ") : (s.error ?? "Nothing to import");
      if (s.failed && s.error) console.warn("[import]", s.error);
      // Show what just arrived.
      if (s.import) this.query = { ...this.query, import: s.import.id, album: null };
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

  #recovered = false;

  /**
   * Once per launch: pick up originals in the library folder that the catalog doesn't know
   * (e.g. after the catalog was lost), with their edits from sidecars.
   */
  async recover() {
    if (this.#recovered) return;
    this.#recovered = true;
    this.restoring = { done: 0, total: 0 };
    try {
      const s = await api.library.recover();
      if (!s.added) return;
      this.message =
        `Restored ${s.added} photo${s.added === 1 ? "" : "s"} from the library folder` +
        (s.restored ? `, ${s.restored} with edits` : "");
      await this.refresh();
    } catch (e) {
      this.message = String(e);
    } finally {
      this.restoring = null;
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

  /** Move keyboard focus to `id` (from arrow keys), extending the selection with Shift. */
  moveTo(id: number | null, extend: boolean) {
    if (id === null) return;
    if (extend) {
      this.selected.add(id);
      this.focus = id;
    } else this.select(id);
  }

  /** Photos an action applies to: the selection, else the focused photo. */
  targets(): number[] {
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

  // ---- albums ----

  private async albumOp<T>(op: () => Promise<T>): Promise<T | undefined> {
    try {
      return await op();
    } catch (e) {
      this.message = String(e);
    } finally {
      await this.refresh();
    }
  }

  /** Create an album, optionally holding `images`. Returns its id. */
  createAlbum(name: string, images: number[] = []) {
    return this.albumOp(async () => {
      const id = await api.albums.create(name, images);
      if (images.length) this.message = `Added ${images.length} photo${images.length === 1 ? "" : "s"} to “${name.trim()}”`;
      return id;
    });
  }

  renameAlbum(id: number, name: string) {
    return this.albumOp(() => api.albums.rename(id, name));
  }

  async deleteAlbum(id: number) {
    const a = this.albums.find((x) => x.id === id);
    const ok = await ask(`Delete the album “${a?.name ?? ""}”? The photos stay in your library.`, {
      title: "Delete album",
      kind: "warning",
      okLabel: "Delete",
    });
    if (ok) await this.albumOp(() => api.albums.delete(id));
  }

  addToAlbum(id: number, images = this.targets()) {
    if (!images.length) return;
    const name = this.albums.find((a) => a.id === id)?.name ?? "album";
    return this.albumOp(async () => {
      const n = await api.albums.add(id, images);
      const skipped = images.length - n;
      this.message =
        `Added ${n} photo${n === 1 ? "" : "s"} to “${name}”` + (skipped ? ` · ${skipped} already there` : "");
    });
  }

  /** Remove the selected photos from the album being shown. */
  removeFromAlbum() {
    const v = this.view;
    const ids = this.targets();
    if (v.kind !== "album" || !ids.length) return;
    return this.albumOp(async () => {
      await api.albums.remove(v.id, ids);
      this.selected.clear();
    });
  }

  // ---- dragging photos onto albums ----

  /** Start dragging from a grid cell: the selection if the cell is in it, else just that photo. */
  startDrag(id: number, x: number, y: number) {
    if (!this.selected.has(id)) this.select(id);
    const ids = this.images.filter((i) => this.selected.has(i.id)).map((i) => i.id);
    this.drag = { ids, x, y, over: null };
    const move = (e: PointerEvent) => {
      if (!this.drag) return;
      const el = document.elementFromPoint(e.clientX, e.clientY)?.closest<HTMLElement>("[data-album]");
      this.drag = { ...this.drag, x: e.clientX, y: e.clientY, over: el ? Number(el.dataset.album) : null };
    };
    const end = (e: Event) => {
      window.removeEventListener("pointermove", move);
      window.removeEventListener("pointerup", end);
      window.removeEventListener("keydown", cancel, true);
      const d = this.drag;
      this.drag = null;
      if (d && d.over !== null && e.type === "pointerup") this.addToAlbum(d.over, d.ids);
    };
    const cancel = (e: KeyboardEvent) => {
      if (e.key !== "Escape") return;
      e.stopPropagation();
      end(e);
    };
    window.addEventListener("pointermove", move);
    window.addEventListener("pointerup", end);
    window.addEventListener("keydown", cancel, true);
  }

  /** In Edit: open the photo `delta` places from the open one, within the current view. */
  step(delta: number) {
    if (!this.images.length) return;
    // From the photo asked for last, so quick presses don't step from one still loading.
    const from = session.opening ?? session.image?.id;
    const cur = this.images.findIndex((i) => i.id === from);
    const i = cur < 0 ? 0 : Math.max(0, Math.min(this.images.length - 1, cur + delta));
    const id = this.images[i].id;
    if (id === from) return;
    this.select(id);
    session.open(id);
  }

  /** Open a photo (default: the focused one) in Edit. */
  open(id = this.focus) {
    if (id === null) return;
    this.focus = id;
    app.mode = "edit";
    if ((session.opening ?? session.image?.id) !== id) session.open(id);
  }
}

export const library = new LibraryState();
