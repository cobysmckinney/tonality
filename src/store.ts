import { create } from "zustand";
import { listen } from "@tauri-apps/api/event";
import { open as openDialog } from "@tauri-apps/plugin-dialog";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { Adjustments, DEFAULTS, describeChange, GEOMETRY, isAsShot, sameAdjustments } from "./adjustments";
import {
  api,
  ExportJob,
  ExportPlan,
  ExportSettings,
  ExportSummary,
  Flag,
  Frame,
  History,
  ImportSummary,
  Overview,
  Photo,
  Progress,
  Scan,
  View,
  Volume,
} from "./api";
import { plural } from "./format";

export type Filter = "all" | "picks" | "unrejected" | "rejects";

export interface Toast {
  id: number;
  text: string;
  tone?: "error";
  action?: { label: string; run: () => void };
}

/**
 * Something that takes long enough to need saying so: shown in the title
 * bar until it ends.
 */
export interface Activity {
  id: number;
  /** What is happening, as a present participle: "Pasting edits". */
  label: string;
  done: number;
  /** How many there are to do; null when it can't be counted, only waited for. */
  total: number | null;
  /** Present when the work can be stopped part-way. */
  stop?: () => void;
}

export type ImportState =
  | { phase: "scanning"; progress: Progress | null }
  | { phase: "review"; scan: Scan; chosen: Set<number> }
  | { phase: "importing"; scan: Scan; progress: Progress }
  /** Only shown when some photos could not be imported. */
  | { phase: "done"; summary: ImportSummary };

export type ExportState =
  /** Choosing. The plan says exactly what running it would write, and where. */
  | { phase: "setup"; job: ExportJob; plan: ExportPlan }
  | { phase: "exporting"; progress: Progress }
  /** Always shown, so there is no doubt about what was written. */
  | { phase: "done"; summary: ExportSummary };

export type SidePanel = "adjust" | "crop" | "history" | "info";

/** Where undo and redo would land in a history, if anywhere. */
export function neighbours(history: History | null): { undo: number | null; redo: number | null } {
  const steps = history?.steps ?? [];
  const at = steps.findIndex((step) => step.id === history?.headId);
  return { undo: steps[at - 1]?.id ?? null, redo: at >= 0 ? (steps[at + 1]?.id ?? null) : null };
}

/** The photo open for editing and the state of its edit session. */
export interface EditorState {
  /** The photo being loaded or edited; follows `openId`. */
  photoId: number | null;
  /** True once the photo is on the GPU and frames can be drawn. */
  ready: boolean;
  size: { width: number; height: number } | null;
  /** What the sliders show right now, including a drag still in progress. */
  adjustments: Adjustments;
  /** The adjustments as of the newest recorded step; differs from the above mid-drag. */
  committed: Adjustments;
  /** The photo's steps and branches, kept in the library. */
  history: History | null;
  showOriginal: boolean;
  showClipping: boolean;
  histogram: Uint32Array | null;
  highlightsClipped: boolean;
  shadowsClipped: boolean;
}

const idleEditor: EditorState = {
  photoId: null,
  ready: false,
  size: null,
  adjustments: DEFAULTS,
  committed: DEFAULTS,
  history: null,
  showOriginal: false,
  showClipping: false,
  histogram: null,
  highlightsClipped: false,
  shadowsClipped: false,
};

export interface ConfirmRequest {
  title: string;
  body: string;
  confirmLabel: string;
  resolve: (confirmed: boolean) => void;
}

export interface MenuItem {
  label: string;
  run?: () => void;
  danger?: boolean;
  checked?: boolean;
  disabled?: boolean;
  submenu?: MenuEntry[];
}
export type MenuEntry = MenuItem | "separator";

interface State {
  overview: Overview | null;
  view: View;
  /** Every photo in the current view, before the flag filter. */
  photos: Photo[];
  loaded: boolean;
  filter: Filter;
  selection: Set<number>;
  /** Where a shift-click or shift-arrow range starts. */
  anchor: number | null;
  /** The photo keyboard navigation moves from; the last one clicked. */
  cursor: number | null;
  /** The photo open in the viewer. */
  openId: number | null;
  thumbSize: number;
  squareThumbs: boolean;
  /** What the editor's side panel shows. */
  sidePanel: SidePanel;
  editor: EditorState;
  /** Edits copied from one photo, ready to paste onto others. */
  clipboard: Adjustments | null;
  /** The crop tool's chosen shape: its name, and width over height (null leaves it free). */
  cropShape: string;
  cropAspect: number | null;
  volumes: Volume[];
  importState: ImportState | null;
  exportState: ExportState | null;
  toasts: Toast[];
  /** Work in progress, oldest first. */
  activities: Activity[];
  confirmRequest: ConfirmRequest | null;
  renamingAlbum: number | null;
  menu: { x: number; y: number; entries: MenuEntry[] } | null;

  init: () => Promise<void>;
  reload: () => Promise<void>;
  setView: (view: View) => Promise<void>;
  setFilter: (filter: Filter) => void;
  setThumbSize: (size: number) => void;
  toggleSquareThumbs: () => void;
  setSidePanel: (panel: SidePanel) => void;
  setCropShape: (shape: string, aspect: number | null) => void;

  loadEditor: (id: number) => Promise<void>;
  leaveEditor: () => Promise<void>;
  /** Changes adjustments live, e.g. while a slider is dragged. */
  adjust: (change: Partial<Adjustments>) => void;
  /**
   * Ends a change: what happened since the last step is recorded as a new
   * step in the photo's history, named `label` or after what changed.
   */
  commitAdjust: (label?: string) => void;
  undo: () => Promise<void>;
  redo: () => Promise<void>;
  /** Jumps to any step of the current branch. */
  goToStep: (stepId: number) => Promise<void>;
  /** Starts a branch at a step (the current one by default) and switches to it. */
  newBranch: (stepId?: number) => Promise<void>;
  switchBranch: (branchId: number) => Promise<void>;
  renameBranch: (branchId: number, name: string) => Promise<void>;
  deleteBranch: (branchId: number) => Promise<void>;
  resetEdits: () => void;
  setShowOriginal: (show: boolean) => void;
  toggleClipping: () => void;
  noteFrame: (frame: Frame) => void;
  copyEdits: (id: number) => Promise<void>;
  pasteEdits: (ids: number[]) => Promise<void>;
  revertEdits: (ids: number[]) => Promise<void>;

  click: (id: number, mods: { range: boolean; toggle: boolean }) => void;
  moveCursor: (id: number, extend: boolean) => void;
  selectAll: () => void;
  clearSelection: () => void;
  openPhoto: (id: number) => void;
  closePhoto: () => void;

  toggleFavorite: (ids: number[]) => Promise<void>;
  toggleFlag: (ids: number[], flag: Flag) => Promise<void>;
  trash: (ids: number[]) => Promise<void>;
  restore: (ids: number[]) => Promise<void>;
  purge: (ids: number[]) => Promise<void>;
  trashRejected: () => Promise<void>;

  newAlbum: (ids: number[]) => Promise<void>;
  renameAlbum: (id: number, name: string) => Promise<void>;
  setRenamingAlbum: (id: number | null) => void;
  deleteAlbum: (id: number) => Promise<void>;
  addToAlbum: (albumId: number, ids: number[]) => Promise<void>;
  removeFromAlbum: (albumId: number, ids: number[]) => Promise<void>;

  importFiles: () => Promise<void>;
  importFolder: () => Promise<void>;
  startImport: (paths: string[], source?: string) => Promise<void>;
  toggleChosen: (indices: number[], chosen: boolean) => void;
  runImport: () => Promise<void>;
  dismissImport: () => Promise<void>;

  /** Opens the export sheet for these photos, with the settings used last time. */
  startExport: (ids: number[]) => Promise<void>;
  /** Changes the export being set up: its settings, which branches it takes, or what the files are called. */
  changeExport: (change: { settings?: Partial<ExportSettings> } & Partial<Pick<ExportJob, "branches" | "name">>) => Promise<void>;
  chooseExportFolder: () => Promise<void>;
  runExport: () => Promise<void>;
  dismissExport: () => Promise<void>;
  /** Shows a file in the system's file manager. */
  reveal: (path: string) => Promise<void>;

  toast: (toast: Omit<Toast, "id">) => void;
  dismissToast: (id: number) => void;
  confirm: (request: Omit<ConfirmRequest, "resolve">) => Promise<boolean>;
  openMenu: (x: number, y: number, entries: MenuEntry[]) => void;
  closeMenu: () => void;
}

const sameView = (a: View, b: View) => a.kind === b.kind && (a.kind !== "album" || a.id === (b as typeof a).id);

function applyFilter(photos: Photo[], filter: Filter, view: View): Photo[] {
  if (filter === "all" || view.kind === "deleted") return photos;
  return photos.filter((p) =>
    filter === "picks" ? p.flag === 1 : filter === "rejects" ? p.flag === -1 : p.flag !== -1,
  );
}

let cachedVisible: { photos: Photo[]; filter: Filter; view: View; result: Photo[] } | null = null;

/** The photos the grid shows: the current view narrowed by the flag filter. */
export function visiblePhotos(state: Pick<State, "photos" | "filter" | "view">): Photo[] {
  const { photos, filter, view } = state;
  if (cachedVisible?.photos !== photos || cachedVisible.filter !== filter || cachedVisible.view !== view) {
    cachedVisible = { photos, filter, view, result: applyFilter(photos, filter, view) };
  }
  return cachedVisible.result;
}

/** The selection if `id` is part of it, otherwise just `id`. What a right-click or shortcut acts on. */
export function targetOf(state: Pick<State, "selection">, id: number): number[] {
  return state.selection.has(id) ? [...state.selection] : [id];
}

const IMAGE_EXTENSIONS = ["jpg", "jpeg", "png", "tif", "tiff", "webp", "heic", "heif"];
// prettier-ignore
const RAW_EXTENSIONS = ["ari", "arw", "cr2", "cr3", "crm", "crw", "dcr", "dcs", "dng", "erf", "iiq", "kdc", "mef", "mos", "mrw", "nef", "nrw", "orf", "ori", "pef", "raf", "raw", "rw2", "rwl", "srw", "3fr", "fff", "x3f", "qtk"];

let nextToastId = 1;
/** Counts export plans asked for, so a slow answer never overwrites a newer one. */
let planRequest = 0;

export const useStore = create<State>((set, get) => {
  /** Runs a backend change; on failure, says so and resyncs with the library. */
  async function attempt(action: () => Promise<unknown>): Promise<boolean> {
    try {
      await action();
      return true;
    } catch (error) {
      get().toast({ text: String(error), tone: "error" });
      await get().reload();
      return false;
    }
  }

  const refreshOverview = async () => set({ overview: await api.getOverview() });

  let nextActivity = 1;
  const beginActivity = (label: string, total: number | null = null, stop?: () => void): number => {
    const id = nextActivity++;
    set((s) => ({ activities: [...s.activities, { id, label, done: 0, total, stop }] }));
    return id;
  };
  const advanceActivity = (id: number, done: number) =>
    set((s) => ({ activities: s.activities.map((a) => (a.id === id ? { ...a, done } : a)) }));
  const endActivity = (id: number) => set((s) => ({ activities: s.activities.filter((a) => a.id !== id) }));
  /** Shows an activity for as long as `task` runs. */
  const during = async <T,>(label: string, task: () => Promise<T>): Promise<T> => {
    const id = beginActivity(label);
    try {
      return await task();
    } finally {
      endActivity(id);
    }
  };
  /** The activity that `edits-progress` events belong to. */
  let editsActivity: number | null = null;

  /**
   * Applies one recipe to many photos, with progress and a way to stop.
   * Returns the photos that were done and their new versions.
   */
  const applyToMany = async (activity: string, ids: number[], recipe: Adjustments, step: string, keepCrop: boolean) => {
    const id = beginActivity(activity, ids.length, () => void api.cancelEdits());
    try {
      // Waits its turn behind anything the editor is still saving.
      const result = await inTurn(() => {
        editsActivity = id;
        return api.applyEdits(ids, recipe, step, keepCrop);
      });
      const done = ids.slice(0, result.versions.length);
      return { done, versionOf: new Map(done.map((photo, i) => [photo, result.versions[i]])), cancelled: result.cancelled };
    } finally {
      editsActivity = null;
      endActivity(id);
    }
  };

  // Opening, saving and closing photos in the editor happen strictly in turn.
  let editorQueue: Promise<unknown> = Promise.resolve();
  const inTurn = <T,>(task: () => Promise<T>): Promise<T> => {
    const result = editorQueue.then(task, task);
    editorQueue = result.catch(() => undefined);
    return result;
  };
  // Recording a step is instant; redrawing the photo's thumbnail is not, so
  // that waits for a pause in the editing.
  let redrawTimer: ReturnType<typeof setTimeout> | undefined;
  let needsRedraw: number | null = null;
  const redrawNow = async () => {
    clearTimeout(redrawTimer);
    const id = needsRedraw;
    if (id === null) return;
    needsRedraw = null;
    try {
      const [version] = await api.refreshRendered([id]);
      patch([id], (p) => ({ ...p, version }));
    } catch (error) {
      get().toast({ text: `Couldn’t update the thumbnail: ${error}`, tone: "error" });
    }
  };
  const redrawSoon = (id: number) => {
    needsRedraw = id;
    clearTimeout(redrawTimer);
    redrawTimer = setTimeout(() => void inTurn(redrawNow), 700);
  };

  const setEditor = (change: Partial<EditorState>) => set((s) => ({ editor: { ...s.editor, ...change } }));

  /** Takes in a history the library just returned for the open photo. */
  const noteHistory = (photoId: number, history: History, checkOut: boolean) => {
    if (get().editor.photoId !== photoId) return;
    // After a jump or a branch switch the sliders follow; after recording a
    // step they are left alone, since another drag may already be under way.
    setEditor(checkOut ? { history, adjustments: history.adjustments, committed: history.adjustments } : { history });
    patch([photoId], (p) => ({ ...p, edited: !isAsShot(history.adjustments), branches: history.branches.length }));
    redrawSoon(photoId);
  };

  /** Runs a history change on the open photo once anything pending has been recorded. */
  const changeHistory = (change: (photoId: number, history: History) => Promise<History | null>) => {
    get().commitAdjust();
    return inTurn(async () => {
      const { photoId, history } = get().editor;
      if (photoId === null || !history) return;
      try {
        const next = await change(photoId, history);
        if (next) noteHistory(photoId, next, true);
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
      }
    });
  };

  /** Before the editor moves on: record what is pending and bring the thumbnail up to date. */
  const settle = () => {
    get().commitAdjust();
    if (needsRedraw !== null) void inTurn(redrawNow);
  };

  const patch = (ids: number[], change: (photo: Photo) => Photo) => {
    const wanted = new Set(ids);
    set((s) => ({ photos: s.photos.map((p) => (wanted.has(p.id) ? change(p) : p)) }));
  };

  /** Drops photos from the current view, keeping selection and viewer sensible. */
  const removeFromView = (ids: number[]) => {
    const gone = new Set(ids);
    const state = get();
    let openId = state.openId;
    if (openId !== null && gone.has(openId)) {
      // Step to the nearest surviving neighbour, preferring the next photo.
      const visible = visiblePhotos(state);
      const at = visible.findIndex((p) => p.id === openId);
      const next = visible.slice(at + 1).find((p) => !gone.has(p.id));
      const previous = visible.slice(0, at).reverse().find((p) => !gone.has(p.id));
      openId = (next ?? previous)?.id ?? null;
    }
    set({
      photos: state.photos.filter((p) => !gone.has(p.id)),
      selection: new Set([...state.selection].filter((id) => !gone.has(id))),
      cursor: state.cursor !== null && gone.has(state.cursor) ? openId : state.cursor,
      anchor: state.anchor !== null && gone.has(state.anchor) ? null : state.anchor,
      openId,
    });
  };

  return {
    overview: null,
    view: { kind: "library" },
    photos: [],
    loaded: false,
    filter: "all",
    selection: new Set(),
    anchor: null,
    cursor: null,
    openId: null,
    thumbSize: 180,
    squareThumbs: true,
    sidePanel: "adjust",
    editor: idleEditor,
    clipboard: null,
    cropShape: "Free",
    cropAspect: null,
    volumes: [],
    importState: null,
    exportState: null,
    toasts: [],
    activities: [],
    confirmRequest: null,
    renamingAlbum: null,
    menu: null,

    async init() {
      await get().reload();
      set({ volumes: await api.listVolumes() });

      void listen<Volume[]>("volumes-changed", ({ payload }) => {
        const known = new Set(get().volumes.map((v) => v.path));
        set({ volumes: payload });
        for (const card of payload.filter((v) => !known.has(v.path))) {
          get().toast({
            text: `Camera card “${card.name}” connected`,
            action: { label: "Review photos", run: () => void get().startImport([card.path], card.name) },
          });
        }
      });
      void listen<Progress>("scan-progress", ({ payload }) => {
        if (get().importState?.phase === "scanning") set({ importState: { phase: "scanning", progress: payload } });
      });
      void listen<Progress>("import-progress", ({ payload }) => {
        const current = get().importState;
        if (current?.phase === "importing") set({ importState: { ...current, progress: payload } });
      });
      void listen<Progress>("edits-progress", ({ payload }) => {
        if (editsActivity !== null) advanceActivity(editsActivity, payload.done);
      });
      void listen<Progress>("export-progress", ({ payload }) => {
        if (get().exportState?.phase === "exporting") set({ exportState: { phase: "exporting", progress: payload } });
      });
    },

    async reload() {
      const view = get().view;
      const [overview, photos] = await Promise.all([api.getOverview(), api.listPhotos(view)]);
      if (!sameView(view, get().view)) return;
      const present = new Set(photos.map((p) => p.id));
      set((s) => ({
        overview,
        photos,
        loaded: true,
        selection: new Set([...s.selection].filter((id) => present.has(id))),
        openId: s.openId !== null && present.has(s.openId) ? s.openId : null,
      }));
    },

    async setView(view) {
      set({ view, photos: [], loaded: false, filter: "all", selection: new Set(), anchor: null, cursor: null, openId: null });
      await get().reload();
    },

    setFilter: (filter) => set({ filter, selection: new Set(), anchor: null, cursor: null }),
    setThumbSize: (thumbSize) => set({ thumbSize }),
    toggleSquareThumbs: () => set((s) => ({ squareThumbs: !s.squareThumbs })),
    setSidePanel: (sidePanel) => set({ sidePanel }),
    setCropShape: (cropShape, cropAspect) => set({ cropShape, cropAspect }),

    loadEditor(id) {
      settle();
      return inTurn(async () => {
        if (get().openId !== id) return;
        const { showClipping } = get().editor;
        // A crop shape chosen for one photo doesn't carry over to the next.
        set({ editor: { ...idleEditor, photoId: id, showClipping }, cropShape: "Free", cropAspect: null });
        try {
          const photo = await api.openEditor(id);
          if (get().openId !== id) return;
          setEditor({
            ready: true,
            size: { width: photo.width, height: photo.height },
            history: photo.history,
            adjustments: photo.history.adjustments,
            committed: photo.history.adjustments,
          });
        } catch (error) {
          get().toast({ text: String(error), tone: "error" });
        }
      });
    },

    leaveEditor() {
      settle();
      return inTurn(async () => {
        // Another photo may have been opened in the meantime; then there is nothing to close.
        if (get().openId !== null) return;
        set({ editor: idleEditor });
        await api.closeEditor();
      });
    },

    adjust(change) {
      const { editor } = get();
      if (editor.ready) setEditor({ adjustments: { ...editor.adjustments, ...change } });
    },

    commitAdjust(label) {
      const { photoId, ready, adjustments, committed } = get().editor;
      if (photoId === null || !ready || sameAdjustments(adjustments, committed)) return;
      const name = label ?? describeChange(committed, adjustments);
      setEditor({ committed: adjustments });
      void inTurn(async () => {
        try {
          noteHistory(photoId, await api.historyCommit(photoId, adjustments, name), false);
        } catch (error) {
          get().toast({ text: `Couldn’t save your edit: ${error}`, tone: "error" });
        }
      });
    },

    undo: () =>
      changeHistory(async (photoId, history) => {
        const target = neighbours(history).undo;
        return target === null ? null : api.historyGoto(photoId, target);
      }),

    redo: () =>
      changeHistory(async (photoId, history) => {
        const target = neighbours(history).redo;
        return target === null ? null : api.historyGoto(photoId, target);
      }),

    goToStep: (stepId) => changeHistory((photoId) => api.historyGoto(photoId, stepId)),

    newBranch: (stepId) =>
      changeHistory((photoId, history) => api.historyBranch(photoId, stepId ?? history.headId, "")),

    switchBranch: (branchId) => changeHistory((photoId) => api.historySwitch(photoId, branchId)),

    renameBranch: (branchId, name) => changeHistory((photoId) => api.historyRenameBranch(photoId, branchId, name)),

    async deleteBranch(branchId) {
      const branch = get().editor.history?.branches.find((b) => b.id === branchId);
      const confirmed = await get().confirm({
        title: `Delete the branch “${branch?.name ?? ""}”?`,
        body: "The edits made only on this branch are removed. The photo and its other branches are not affected.",
        confirmLabel: "Delete branch",
      });
      if (confirmed) await changeHistory((photoId) => api.historyDeleteBranch(photoId, branchId));
    },

    resetEdits() {
      get().adjust(DEFAULTS);
      get().commitAdjust("Reset all");
    },

    setShowOriginal: (showOriginal) => setEditor({ showOriginal }),
    toggleClipping: () => setEditor({ showClipping: !get().editor.showClipping }),
    noteFrame: (frame) =>
      setEditor({
        histogram: frame.histogram,
        highlightsClipped: frame.highlightsClipped,
        shadowsClipped: frame.shadowsClipped,
      }),

    async copyEdits(id) {
      const { editor } = get();
      const open = editor.photoId === id && editor.ready;
      try {
        set({ clipboard: open ? editor.adjustments : await api.getEdits(id) });
        get().toast({ text: "Edits copied" });
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
      }
    },

    async pasteEdits(ids) {
      const { clipboard, editor } = get();
      if (!clipboard || ids.length === 0) return;
      if (ids.length === 1 && ids[0] === editor.photoId && editor.ready) {
        // In the editor a paste is an ordinary step in the history. The
        // crop is left alone: it belongs to this photo's framing.
        const own = Object.fromEntries(GEOMETRY.map((key) => [key, editor.adjustments[key]]));
        get().adjust({ ...clipboard, ...own });
        get().commitAdjust("Paste edits");
        return;
      }
      try {
        const { done, versionOf, cancelled } = await applyToMany("Pasting edits", ids, clipboard, "Paste edits", true);
        patch(done, (p) => ({
          ...p,
          version: versionOf.get(p.id) ?? p.version,
          // Close enough until the next reload: a photo with only a crop of its own counts as edited too.
          edited: p.edited || !isAsShot(clipboard),
          branches: Math.max(1, p.branches),
        }));
        get().toast({
          text: cancelled
            ? `Stopped: edits applied to ${done.length} of ${plural(ids.length, "photo")}`
            : `Edits applied to ${plural(ids.length, "photo")}`,
        });
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
        await get().reload();
      }
    },

    async revertEdits(ids) {
      const edited = get().photos.filter((p) => ids.includes(p.id) && p.edited).map((p) => p.id);
      if (edited.length === 0) return;
      try {
        // A step like any other, so the edits can still be brought back from the history.
        const { done, versionOf, cancelled } = await applyToMany("Reverting", edited, DEFAULTS, "Revert to original", false);
        patch(done, (p) => ({ ...p, version: versionOf.get(p.id) ?? p.version, edited: false }));
        get().toast({
          text: cancelled
            ? `Stopped: reverted ${done.length} of ${plural(edited.length, "photo")}`
            : `Reverted ${plural(edited.length, "photo")} to original`,
        });
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
        await get().reload();
      }
    },

    click(id, { range, toggle }) {
      const state = get();
      if (range && state.anchor !== null) {
        const ids = visiblePhotos(state).map((p) => p.id);
        const [from, to] = [ids.indexOf(state.anchor), ids.indexOf(id)].sort((a, b) => a - b);
        if (from >= 0) {
          set({ selection: new Set(ids.slice(from, to + 1)), cursor: id });
          return;
        }
      }
      if (toggle) {
        const selection = new Set(state.selection);
        if (!selection.delete(id)) selection.add(id);
        set({ selection, anchor: id, cursor: id });
      } else {
        set({ selection: new Set([id]), anchor: id, cursor: id });
      }
    },

    moveCursor(id, extend) {
      if (extend) get().click(id, { range: true, toggle: false });
      else set({ selection: new Set([id]), anchor: id, cursor: id });
    },

    selectAll: () => set((s) => ({ selection: new Set(visiblePhotos(s).map((p) => p.id)) })),
    clearSelection: () => set({ selection: new Set(), anchor: null }),
    openPhoto: (id) => set({ openId: id, selection: new Set([id]), anchor: id, cursor: id }),
    closePhoto: () => set({ openId: null }),

    async toggleFavorite(ids) {
      const chosen = get().photos.filter((p) => ids.includes(p.id));
      if (chosen.length === 0) return;
      const favorite = !chosen.every((p) => p.favorite);
      if (!favorite && get().view.kind === "favorites") removeFromView(ids);
      else patch(ids, (p) => ({ ...p, favorite }));
      if (await attempt(() => api.setFavorite(ids, favorite))) await refreshOverview();
    },

    // Pressing the same flag again clears it, like the heart.
    async toggleFlag(ids, flag) {
      const chosen = get().photos.filter((p) => ids.includes(p.id));
      if (chosen.length === 0) return;
      const next: Flag = flag !== 0 && chosen.every((p) => p.flag === flag) ? 0 : flag;
      patch(ids, (p) => ({ ...p, flag: next }));
      await attempt(() => api.setFlag(ids, next));
    },

    async trash(ids) {
      if (ids.length === 0) return;
      removeFromView(ids);
      if (!(await attempt(() => api.trashPhotos(ids)))) return;
      await refreshOverview();
      get().toast({
        text: `Moved ${plural(ids.length, "photo")} to Recently Deleted`,
        action: { label: "Undo", run: () => void get().restore(ids) },
      });
    },

    async restore(ids) {
      if (get().view.kind === "deleted") removeFromView(ids);
      if (await attempt(() => api.restorePhotos(ids))) await get().reload();
    },

    async purge(ids) {
      if (ids.length === 0) return;
      const confirmed = await get().confirm({
        title: `Delete ${plural(ids.length, "photo")} permanently?`,
        body: "The original files are removed from your library folder. This can’t be undone.",
        confirmLabel: "Delete permanently",
      });
      if (!confirmed) return;
      removeFromView(ids);
      if (await during("Deleting", () => attempt(() => api.purgePhotos(ids)))) await refreshOverview();
    },

    async trashRejected() {
      const rejected = get().photos.filter((p) => p.flag === -1).map((p) => p.id);
      if (rejected.length === 0) {
        get().toast({ text: "No rejected photos here" });
        return;
      }
      await get().trash(rejected);
    },

    async newAlbum(ids) {
      let album = 0;
      const created = await attempt(async () => {
        album = await api.createAlbum("Untitled album", ids);
      });
      if (!created) return;
      await refreshOverview();
      set({ renamingAlbum: album });
    },

    async renameAlbum(id, name) {
      set({ renamingAlbum: null });
      const current = get().overview?.albums.find((a) => a.id === id);
      if (!name.trim() || name.trim() === current?.name) return;
      if (await attempt(() => api.renameAlbum(id, name))) await refreshOverview();
    },

    setRenamingAlbum: (renamingAlbum) => set({ renamingAlbum }),

    async deleteAlbum(id) {
      const album = get().overview?.albums.find((a) => a.id === id);
      const confirmed = await get().confirm({
        title: `Delete the album “${album?.name ?? ""}”?`,
        body: "Its photos stay in your library.",
        confirmLabel: "Delete album",
      });
      if (!confirmed) return;
      if (!(await attempt(() => api.deleteAlbum(id)))) return;
      const view = get().view;
      if (view.kind === "album" && view.id === id) await get().setView({ kind: "library" });
      else await refreshOverview();
    },

    async addToAlbum(albumId, ids) {
      if (!(await attempt(() => api.addToAlbum(albumId, ids)))) return;
      await refreshOverview();
      const album = get().overview?.albums.find((a) => a.id === albumId);
      get().toast({ text: `Added ${plural(ids.length, "photo")} to ${album?.name ?? "the album"}` });
    },

    async removeFromAlbum(albumId, ids) {
      const view = get().view;
      if (view.kind === "album" && view.id === albumId) removeFromView(ids);
      if (await attempt(() => api.removeFromAlbum(albumId, ids))) await refreshOverview();
    },

    async importFiles() {
      const picked = await openDialog({
        multiple: true,
        title: "Import photos",
        filters: [{ name: "Photos", extensions: [...RAW_EXTENSIONS, ...IMAGE_EXTENSIONS] }],
      });
      if (picked?.length) await get().startImport(picked);
    },

    async importFolder() {
      const picked = await openDialog({ directory: true, multiple: true, title: "Import a folder" });
      if (picked?.length) await get().startImport(picked);
    },

    async startImport(paths, source) {
      const busy = get().importState?.phase;
      if (busy === "scanning" || busy === "importing") return;
      set({ importState: { phase: "scanning", progress: null }, menu: null });
      try {
        const scan = await api.scanImport(paths, source);
        if (scan.items.length === 0) {
          set({ importState: null });
          get().toast({ text: `No photos found in ${scan.source}` });
          return;
        }
        const chosen = new Set(scan.items.filter((i) => i.status !== "duplicate").map((i) => i.index));
        set({ importState: { phase: "review", scan, chosen } });
      } catch (error) {
        set({ importState: null });
        get().toast({ text: String(error), tone: "error" });
      }
    },

    toggleChosen(indices, chosen) {
      const current = get().importState;
      if (current?.phase !== "review") return;
      const next = new Set(current.chosen);
      for (const index of indices) {
        if (chosen) next.add(index);
        else next.delete(index);
      }
      set({ importState: { ...current, chosen: next } });
    },

    async runImport() {
      const current = get().importState;
      if (current?.phase !== "review" || current.chosen.size === 0) return;
      const indices = [...current.chosen].sort((a, b) => a - b);
      set({ importState: { phase: "importing", scan: current.scan, progress: { done: 0, total: indices.length } } });
      try {
        const summary = await api.runImport(current.scan.sessionId, indices);
        set({ importState: summary.failed.length > 0 ? { phase: "done", summary } : null });
        const added = summary.imported + summary.restored;
        if (added > 0) {
          await get().setView({ kind: "imports" });
          get().toast({ text: `Imported ${plural(added, "photo")}${summary.cancelled ? " before stopping" : ""}` });
        } else {
          await get().reload();
        }
      } catch (error) {
        set({ importState: null });
        get().toast({ text: String(error), tone: "error" });
        await get().reload();
      }
    },

    async dismissImport() {
      const phase = get().importState?.phase;
      if (phase === "importing") {
        // The running import notices, finishes the photo in hand, and reports back.
        await api.cancelImport();
      } else if (phase !== "scanning") {
        set({ importState: null });
        await api.discardScan();
      }
    },

    async startExport(ids) {
      if (ids.length === 0 || get().exportState || get().importState) return;
      // An edit still in hand is recorded first: what is on screen is what gets exported.
      settle();
      set({ menu: null });
      const job: ExportJob = { ids, branches: "current", name: null };
      try {
        const plan = await inTurn(() => api.planExport(job, null));
        set({ exportState: { phase: "setup", job, plan } });
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
      }
    },

    async changeExport({ settings: changed, ...jobChange }) {
      const current = get().exportState;
      if (current?.phase !== "setup") return;
      const job = { ...current.job, ...jobChange };
      const settings = { ...current.plan.settings, ...changed };
      // The controls answer at once; the file list follows when the library has worked it out.
      set({ exportState: { ...current, job, plan: { ...current.plan, settings } } });
      // Quality changes nothing about which files are written or what they are called.
      const sameFiles = Object.keys(jobChange).length === 0 && Object.keys(changed ?? {}).every((key) => key === "quality");
      if (sameFiles) return;
      const request = ++planRequest;
      try {
        const plan = await api.planExport(job, settings);
        const latest = get().exportState;
        if (request !== planRequest || latest?.phase !== "setup") return;
        // Quality may have moved on while this was worked out.
        set({ exportState: { ...latest, plan: { ...plan, settings: { ...plan.settings, quality: latest.plan.settings.quality } } } });
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
      }
    },

    async chooseExportFolder() {
      const current = get().exportState;
      if (current?.phase !== "setup") return;
      const folder = await openDialog({ directory: true, title: "Export to", defaultPath: current.plan.folder });
      if (folder) await get().changeExport({ settings: { folder } });
    },

    async runExport() {
      const current = get().exportState;
      if (current?.phase !== "setup") return;
      const { job, plan } = current;
      // Any change still being planned no longer matters.
      planRequest++;
      set({ exportState: { phase: "exporting", progress: { done: 0, total: plan.files.length } } });
      try {
        const summary = await inTurn(() => api.runExport(job, plan.settings));
        set({ exportState: { phase: "done", summary } });
        // The history marks the step that was exported.
        const open = get().editor.photoId;
        if (open !== null && summary.exported.some((file) => file.photoId === open)) {
          const history = await api.getHistory(open);
          if (get().editor.photoId === open) setEditor({ history });
        }
      } catch (error) {
        set({ exportState: null });
        get().toast({ text: `Couldn’t export: ${error}`, tone: "error" });
      }
    },

    async dismissExport() {
      // A running export notices, finishes the photo in hand, and reports what it wrote.
      if (get().exportState?.phase === "exporting") await api.cancelExport();
      else set({ exportState: null });
    },

    async reveal(path) {
      try {
        await revealItemInDir(path);
      } catch {
        get().toast({ text: `${path} is no longer there`, tone: "error" });
      }
    },

    toast(toast) {
      const id = nextToastId++;
      set((s) => ({ toasts: [...s.toasts.slice(-2), { ...toast, id }] }));
      setTimeout(() => get().dismissToast(id), toast.action || toast.tone === "error" ? 8000 : 4000);
    },
    dismissToast: (id) => set((s) => ({ toasts: s.toasts.filter((t) => t.id !== id) })),

    confirm: (request) =>
      new Promise<boolean>((resolve) => {
        set({
          menu: null,
          confirmRequest: {
            ...request,
            resolve: (confirmed) => {
              set({ confirmRequest: null });
              resolve(confirmed);
            },
          },
        });
      }),

    openMenu: (x, y, entries) => set({ menu: { x, y, entries } }),
    closeMenu: () => set({ menu: null }),
  };
});
