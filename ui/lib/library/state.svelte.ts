// Library state: imports, the current photo list, filters and selection.
// Module-level so scroll position and selection survive a trip to Edit and back.
import { listen } from "@tauri-apps/api/event";
import { getCurrentWebview } from "@tauri-apps/api/webview";
import { ask, open } from "@tauri-apps/plugin-dialog";
import { SvelteSet } from "svelte/reactivity";
import { api, RAW_EXTENSIONS } from "$lib/api";
import { app } from "$lib/app.svelte";
import type { AlbumInfo } from "$lib/bindings/AlbumInfo";
import type { EditState } from "$lib/bindings/EditState";
import type { ImageQuery } from "$lib/bindings/ImageQuery";
import type { ImageRow } from "$lib/bindings/ImageRow";
import type { ImportInfo } from "$lib/bindings/ImportInfo";
import { session } from "$lib/edit/session.svelte";
import { dropLibraryThumbs } from "$lib/edit/versions";
import { invalidateThumb } from "$lib/library/thumbs";
import { nextRating, sameJson } from "./culling";
import { arrowSelect, clickSelect, type Mods } from "./selection";

const SIZE_KEY = "tonality.library.thumbSize";
const RAW_FILTER = [
  {
    name: "RAW photos",
    extensions: [...RAW_EXTENSIONS, ...RAW_EXTENSIONS.map((e) => e.toUpperCase())],
  },
];

/** What this system calls the place deleted files go. */
export const BIN = navigator.userAgent.includes("Windows") ? "Recycle Bin" : "Trash";

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
  query = $state<ImageQuery>({
    import: null,
    album: null,
    minRating: 0,
    flag: null,
    sort: "captureTime",
    descending: true,
  });
  selected = new SvelteSet<number>();
  /** Keyboard focus (image id). */
  focus = $state<number | null>(null);
  /** Where a Shift range starts (image id); see `selection.ts`. */
  anchor: number | null = null;
  thumbSize = $state(storedSize());
  importing = $state<{ done: number; total: number } | null>(null);
  /** Stop was asked for: photos being copied finish first. */
  stoppingImport = $state(false);
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
    // Edits in Edit move a photo's HEAD: keep its "edited" mark (filmstrip, grid) in step.
    session.onHead = (id, state) => this.noteHead(id, state);
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
    this.setQuery({
      album: v.kind === "album" ? v.id : null,
      import: v.kind === "import" ? v.id : null,
    });
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

  /** Bumped per refresh: only the newest one's results are shown (filters can change faster
   *  than queries come back). */
  private refreshSeq = 0;

  async refresh(): Promise<void> {
    const seq = ++this.refreshSeq;
    try {
      const [overview, images] = await Promise.all([
        api.library.overview(),
        api.library.images(this.query),
      ]);
      if (seq !== this.refreshSeq) return;
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
      if (this.anchor !== null && !ids.has(this.anchor)) this.anchor = null;
    } catch (e) {
      if (seq === this.refreshSeq) this.message = String(e);
    } finally {
      if (seq === this.refreshSeq) this.loaded = true;
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
    const paths = await open({
      multiple: true,
      directory: false,
      title: "Import photos",
      filters: RAW_FILTER,
    });
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
      if (s.failed) parts.push(`${s.failed} failed${s.error ? ` (${s.error})` : ""}`);
      if (s.skipped) parts.unshift("Import stopped", `${s.skipped} left out`);
      this.message = parts.length ? parts.join(" · ") : (s.error ?? "Nothing to import");
      if (s.failed && s.error) console.warn("[import]", s.error);
      // Show what just arrived.
      if (s.import) this.query = { ...this.query, import: s.import.id, album: null };
      this.scrollTop = 0;
      await this.refresh();
      if (s.images.length) {
        this.selected.clear();
        this.focus = this.anchor = s.images[0];
        this.selected.add(s.images[0]);
        if (opts.open) this.open(s.images[0]);
      }
    } catch (e) {
      this.message = String(e);
    } finally {
      this.importing = null;
      this.stoppingImport = false;
    }
  }

  /** Stop the running import. Photos already copied stay in the library. */
  async stopImport() {
    if (!this.importing || this.stoppingImport) return;
    this.stoppingImport = true;
    await api.library.cancelImport().catch((e) => (this.message = String(e)));
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

  private get ids(): number[] {
    return this.images.map((i) => i.id);
  }

  select(id: number, e?: Mods) {
    clickSelect(this, this.ids, id, e);
  }

  selectAll() {
    for (const i of this.images) this.selected.add(i.id);
  }

  clearSelection() {
    this.selected.clear();
  }

  /** Move keyboard focus to `id` (from arrow keys); with Shift, select from the anchor. */
  moveTo(id: number | null, extend: boolean) {
    if (id !== null) arrowSelect(this, this.ids, id, extend);
  }

  /** Photos an action applies to: the selection, else the focused photo. */
  targets(): number[] {
    if (this.selected.size) return [...this.selected];
    return this.focus !== null ? [this.focus] : [];
  }

  /** A thumbnail showed photo `id`'s shape (width / height): lay it out by that. */
  learnAspect(id: number, aspect: number) {
    const row = this.images.find((i) => i.id === id);
    if (!row || !(aspect > 0) || (row.aspect && Math.abs(row.aspect / aspect - 1) < 0.02)) return;
    this.patch([id], (i) => ({ ...i, aspect }));
  }

  private patch(ids: number[], f: (i: ImageRow) => ImageRow) {
    const set = new Set(ids);
    this.images = this.images.map((i) => (set.has(i.id) ? f(i) : i));
  }

  /**
   * Rate `ids` (default: the selection, else the focused photo); pressing their current rating
   * again clears it. Shown at once, saved behind. Returns the rating given, or null for no photos.
   */
  setRating(rating: number, ids = this.targets()): number | null {
    if (!ids.length) return null;
    const r = nextRating(
      ids.map((id) => this.images.find((i) => i.id === id)?.rating),
      rating,
    );
    this.patch(ids, (i) => ({ ...i, rating: r }));
    api.library.setRating(ids, r).catch((e) => (this.message = String(e)));
    return r;
  }

  /** Flag `ids` (default: as for `setRating`): 1 pick, -1 reject, 0 none. */
  setFlag(flag: number, ids = this.targets()): number | null {
    if (!ids.length) return null;
    this.patch(ids, (i) => ({ ...i, flag }));
    api.library.setFlag(ids, flag).catch((e) => (this.message = String(e)));
    return flag;
  }

  /** Newest `noteHead` per photo, so a late answer can't overwrite a newer one. */
  private headState = new Map<number, EditState>();

  /** Photo `id`'s HEAD now shows `state`: mark it edited, or not, without a refresh. */
  private async noteHead(id: number, state: EditState) {
    this.headState.set(id, state);
    const edited = !sameJson(state, await session.defaultEdit());
    if (this.headState.get(id) !== state) return;
    this.headState.delete(id);
    if (this.images.find((i) => i.id === id)?.edited !== edited)
      this.patch([id], (i) => ({ ...i, edited }));
  }

  /** Take the selected photos back to as-shot, deleting all their versions and steps. */
  async resetHistory() {
    const ids = this.targets();
    const edited = ids.filter((id) => this.images.find((i) => i.id === id)?.head != null);
    if (!edited.length) {
      this.message = ids.length === 1 ? "This photo has no edits" : "These photos have no edits";
      return;
    }
    const what = edited.length === 1 ? "this photo" : `these ${edited.length} photos`;
    const ok = await ask(
      `Reset ${what} to as-shot? Every version and every step of ${edited.length === 1 ? "its" : "their"} history is deleted. This can't be undone.\n\nRatings, flags and albums stay.`,
      { title: "Reset history", kind: "warning", okLabel: "Reset", cancelLabel: "Cancel" },
    );
    if (!ok) return;
    await dropLibraryThumbs(edited);
    try {
      await api.history.reset(edited);
      this.message = `Reset ${edited.length} photo${edited.length === 1 ? "" : "s"} to as-shot`;
    } catch (e) {
      this.message = String(e);
    }
    for (const id of edited) invalidateThumb(id);
    if (session.history && edited.includes(session.history.imageId)) await session.reloadHistory();
    await this.refresh();
  }

  /**
   * Take the selected photos out of the library, moving their files to the system's Trash.
   * Their edits go along in the sidecars, so importing them again from there brings those back.
   */
  async remove() {
    const ids = this.targets();
    if (!ids.length) return;
    const one = ids.length === 1;
    const ok = await ask(
      `Move ${one ? "this photo" : `these ${ids.length} photos`} to the ${BIN}? ` +
        `${one ? "It leaves" : "They leave"} your library, along with ${one ? "its" : "their"} ` +
        `edits, versions, rating and album places.\n\n` +
        `You can get ${one ? "it" : "them"} back from the ${BIN}: import ${one ? "it" : "them"} ` +
        `again from there and the edits come back too.`,
      {
        title: one ? "Remove photo" : "Remove photos",
        kind: "warning",
        okLabel: `Move to ${BIN}`,
        cancelLabel: "Cancel",
      },
    );
    if (!ok) return;
    // Where the first of them was, to leave the focus on the photo that takes its place.
    const at = this.images.findIndex((i) => ids.includes(i.id));
    // A thumbnail still to be saved for one of them mustn't land after it's gone.
    await dropLibraryThumbs(ids);
    try {
      const s = await api.library.remove(ids);
      // Out of the grid first, so no cell asks for a thumbnail that's no longer there.
      const gone = new Set(s.removed);
      this.images = this.images.filter((i) => !gone.has(i.id));
      for (const id of s.removed) this.selected.delete(id);
      // Ids are given out again: nothing may still be held under these.
      for (const id of s.removed) invalidateThumb(id);
      session.forget(s.removed);
      const n = s.removed.length;
      this.message = s.failed
        ? `Couldn’t move ${s.failed} photo${s.failed === 1 ? "" : "s"} to the ${BIN}: ${s.error}`
        : `Moved ${n} photo${n === 1 ? "" : "s"} to the ${BIN}`;
    } catch (e) {
      this.message = String(e);
    }
    await this.refresh();
    if (!this.selected.size && this.images.length) {
      this.focus = this.anchor = this.images[Math.min(Math.max(at, 0), this.images.length - 1)].id;
    }
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
      if (images.length)
        this.message = `Added ${images.length} photo${images.length === 1 ? "" : "s"} to “${name.trim()}”`;
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
        `Added ${n} photo${n === 1 ? "" : "s"} to “${name}”` +
        (skipped ? ` · ${skipped} already there` : "");
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
      const el = document
        .elementFromPoint(e.clientX, e.clientY)
        ?.closest<HTMLElement>("[data-album]");
      this.drag = {
        ...this.drag,
        x: e.clientX,
        y: e.clientY,
        over: el ? Number(el.dataset.album) : null,
      };
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
    if (id !== this.focus) this.focus = this.anchor = id;
    app.mode = "edit";
    if ((session.opening ?? session.image?.id) !== id) session.open(id);
  }
}

export const library = new LibraryState();
