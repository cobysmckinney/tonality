import { create } from "zustand";
import { revealItemInDir } from "@tauri-apps/plugin-opener";
import {
  Adjustments,
  DEFAULTS,
  describeChange,
  Film,
  GEOMETRY,
  isAsShot,
  Mask,
  MaskMode,
  MaskPart,
  Point,
  sameAdjustments,
  Shape,
} from "./adjustments";
import {
  api,
  AppliedEdits,
  ExportJob,
  ExportPlan,
  ExportSettings,
  ExportSummary,
  Flag,
  Frame,
  History,
  ImportSummary,
  LibraryProblem,
  Overview,
  Photo,
  Progress,
  Scan,
  View,
  Volume,
} from "./api";
import { listenAll, settleEach } from "./events";
import { FilmPhoto, FilmSuggestions } from "./filmDetails";
import { plural } from "./format";
import {
  canAdd,
  circleShape,
  foundShapes,
  framingKey,
  isFound,
  MaskStart,
  MAX_MASKS,
  newMask,
  newShape,
  partsToReframe,
  SHAPE_NAMES,
  startKind,
} from "./masks";
import { blend, holds, Preset, settingsFrom } from "./presets";
import { forget } from "./selection";
import { canStartExport, canStartFilm, canStartImport } from "./sheets";

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

export type SidePanel = "adjust" | "crop" | "masks" | "presets" | "history" | "info";

/** How the brush paints. Kept from photo to photo. */
export interface BrushSettings {
  /** 1..100; see `brushRadius`. */
  size: number;
  /** 0..100: how much of the brush's radius fades out. */
  feather: number;
  /** 1..100: how much a stroke covers. */
  strength: number;
  /** Erase rather than paint (Alt does the opposite while held). */
  erase: boolean;
}

/** The preset applied last in this sitting, kept so its amount can still be changed. */
export interface AppliedPreset {
  preset: Preset;
  /** The photo's edits before it was applied: what none of it looks like. */
  base: Adjustments;
  /** Percent: 100 is the preset as it is. */
  amount: number;
  /** What it set the settings it covers to, at that amount. */
  values: Partial<Adjustments>;
  /**
   * The history steps recorded while its amount could still change, and the
   * amount at each. Undo and redo keep it adjustable on these and nowhere else.
   */
  steps: Map<number, number>;
}

/** What the history calls applying a preset, or changing how much of it there is. */
export const presetStepLabel = (preset: Preset, amount: number) =>
  amount === 100 ? `Preset: ${preset.name}` : `Preset: ${preset.name} at ${amount}%`;

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
  /** Why the photo couldn't be opened for editing (a missing or damaged original), if it couldn't. */
  failed: string | null;
  size: { width: number; height: number } | null;
  /** What the sliders show right now, including a drag still in progress. */
  adjustments: Adjustments;
  /** The adjustments as of the newest recorded step; differs from the above mid-drag. */
  committed: Adjustments;
  /** The photo's steps and branches, kept in the library. */
  history: History | null;
  /** A preset being tried on the photo without being applied: the one under the pointer. */
  preview: Preset | null;
  applied: AppliedPreset | null;
  showOriginal: boolean;
  showClipping: boolean;
  /** The mask being worked on in the Masks panel, by id. */
  maskId: number | null;
  /** Which of its parts the photo's overlay edits; a brush part takes the strokes. */
  partIndex: number | null;
  /** Whether the mask being worked on is tinted on the photo. */
  showMask: boolean;
  /** What the backend is looking for in the photo, for a mask to use ("the subject"), while it does. */
  finding: string | null;
  /** Counts the times found parts came back, which can change the picture without changing the recipe. */
  found: number;
  /**
   * Drawing a circle on the photo, for its object to be found: to start a
   * new mask (`mode` null), to add to the mask being worked on, or to
   * `replace` one of its parts. `points` holds the finished circle while
   * the object is found.
   */
  circling: { mode: MaskMode | null; replace?: number; points?: Point[] } | null;
  /** Waiting for a click on the clear film, to set a negative's film base. */
  pickingBase: boolean;
  histogram: Uint32Array | null;
  highlightsClipped: boolean;
  shadowsClipped: boolean;
}

const idleEditor: EditorState = {
  photoId: null,
  ready: false,
  failed: null,
  size: null,
  adjustments: DEFAULTS,
  committed: DEFAULTS,
  history: null,
  preview: null,
  applied: null,
  showOriginal: false,
  showClipping: false,
  maskId: null,
  partIndex: null,
  showMask: true,
  finding: null,
  found: 0,
  circling: null,
  pickingBase: false,
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
  /** A key that does the same, shown at the right. */
  hint?: string;
  run?: () => void;
  danger?: boolean;
  checked?: boolean;
  disabled?: boolean;
  submenu?: MenuEntry[];
}
export type MenuEntry = MenuItem | "separator";

interface State {
  /** Set when the library couldn't open; the app shows why instead of the grid. */
  libraryProblem: LibraryProblem | null;
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
  /** Every preset: the built-in ones, then your own. */
  presets: Preset[];
  /** The starred presets' ids, in the order they were starred. */
  favoritePresets: number[];
  /** The crop tool's chosen shape: its name, and width over height (null leaves it free). */
  cropShape: string;
  cropAspect: number | null;
  brush: BrushSettings;
  volumes: Volume[];
  importState: ImportState | null;
  exportState: ExportState | null;
  /** The film details sheet: the photos it is for, as they are now, and what to offer while typing. */
  filmSheet: { photos: FilmPhoto[]; suggestions: FilmSuggestions } | null;
  /** Counts film details saved, so what shows them knows to look again. */
  filmSaves: number;
  toasts: Toast[];
  /** Work in progress, oldest first. */
  activities: Activity[];
  confirmRequest: ConfirmRequest | null;
  renamingAlbum: number | null;
  menu: { x: number; y: number; entries: MenuEntry[] } | null;
  /** The list of keyboard shortcuts is open. */
  shortcutsOpen: boolean;

  /** Loads what the app starts with; anything that fails to load is reported, and the rest still loads. */
  init: () => Promise<void>;
  /** Starts listening for camera cards and progress from the backend; the returned function stops. */
  subscribe: () => () => void;
  retryLibrary: () => Promise<void>;
  reload: () => Promise<void>;
  setView: (view: View) => Promise<void>;
  setFilter: (filter: Filter) => void;
  setThumbSize: (size: number) => void;
  toggleSquareThumbs: () => void;
  setSidePanel: (panel: SidePanel) => void;
  setCropShape: (shape: string, aspect: number | null) => void;
  setBrush: (change: Partial<BrushSettings>) => void;

  /** Replaces the open photo's masks, live; with a label the change is also a step in the history. */
  setMasks: (masks: Mask[], label?: string) => void;
  /**
   * Starts a mask holding one part of this kind (or the background), and
   * works on it. An object needs a circle drawn first: without one
   * (`drawn`), drawing it begins. When the part has to be found first,
   * returns once that's done.
   */
  addMask: (start: MaskStart, drawn?: Shape) => Promise<void> | void;
  /** Adds a part to the mask being worked on; an object, as `addMask`. */
  addMaskPart: (kind: Shape["kind"], mode: MaskMode, drawn?: Shape) => Promise<void> | void;
  /** Begins drawing a circle on the photo, for its object to be found. */
  startCircle: (circling: NonNullable<EditorState["circling"]>) => void;
  cancelCircle: () => void;
  /** Takes the circle drawn (points on the photo file) and finds the object in it. */
  finishCircle: (points: Point[]) => void;
  /** Changes a mask; with a label the change is also a step in the history. */
  updateMask: (id: number, change: Partial<Mask>, label?: string) => void;
  /** Changes one part of the mask being worked on. */
  updateMaskPart: (index: number, part: MaskPart, label?: string) => void;
  removeMask: (id: number) => void;
  removeMaskPart: (index: number) => void;
  selectMask: (id: number | null, partIndex?: number | null) => void;
  toggleMaskOverlay: () => void;

  loadEditor: (id: number) => Promise<void>;
  leaveEditor: () => Promise<void>;
  /**
   * Finds the subject and sky again in the photo's crop, once the crop is
   * settled (not while the crop tool is open): they are found in the crop
   * alone, so the film holder around a scan isn't taken for either.
   */
  reframe: () => void;
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
  /** Changes the open photo's film settings, as a step named `label`. */
  setFilm: (change: Partial<Film>, label?: string) => void;
  /** Starts or stops waiting for a click on the clear film. */
  setPickingBase: (picking: boolean) => void;
  /** Sets the open photo's film base from a point on its file (0..1 across and down). */
  pickBase: (point: Point) => Promise<void>;
  /** Balances these frames of a roll of film together: a step in each one's history. */
  balanceRoll: (ids: number[]) => Promise<void>;

  /** Shows a preset on the open photo without applying it; null puts the photo's own edits back. */
  previewPreset: (preset: Preset | null) => void;
  /** Lays a preset over these photos' edits: a step in each one's history. */
  applyPreset: (ids: number[], preset: Preset) => Promise<void>;
  /** Changes how much of the preset just applied the open photo takes, in percent. */
  setPresetAmount: (amount: number) => void;
  /** Saves the open photo's edits in these sections of the adjust panel as a preset of your own. */
  savePreset: (name: string, sections: string[]) => Promise<void>;
  renamePreset: (id: number, name: string) => Promise<void>;
  /** Replaces what a preset holds with the open photo's edits, for the same settings. */
  updatePreset: (preset: Preset) => Promise<void>;
  deletePreset: (preset: Preset) => Promise<void>;
  /** Stars a preset so it is listed among the favorites at the top, or takes the star away. */
  toggleFavoritePreset: (id: number) => Promise<void>;
  importPresets: () => Promise<void>;
  exportPreset: (preset: Preset) => Promise<void>;

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
  /** Opens the film details sheet for these photos. */
  editFilmDetails: (ids: number[]) => Promise<void>;
  /** Gives each photo these details, and closes the sheet. */
  saveFilmDetails: (photos: FilmPhoto[]) => Promise<void>;
  closeFilmDetails: () => void;
  /** Shows a file in the system's file manager. */
  reveal: (path: string) => Promise<void>;

  toast: (toast: Omit<Toast, "id">) => void;
  dismissToast: (id: number) => void;
  confirm: (request: Omit<ConfirmRequest, "resolve">) => Promise<boolean>;
  openMenu: (x: number, y: number, entries: MenuEntry[]) => void;
  closeMenu: () => void;
  setShortcutsOpen: (open: boolean) => void;
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

/** The preset whose amount can still be changed: the one applied last, while what it set still stands. */
export function adjustablePreset(state: Pick<State, "editor">): AppliedPreset | null {
  const { applied, adjustments } = state.editor;
  return applied && holds(adjustments, applied.values) ? applied : null;
}

/** The selection if `id` is part of it, otherwise just `id`. What a right-click or shortcut acts on. */
export function targetOf(state: Pick<State, "selection">, id: number): number[] {
  return state.selection.has(id) ? [...state.selection] : [id];
}

let nextToastId = 1;
/** Counts export plans asked for, so a slow answer never overwrites a newer one. */
let planRequest = 0;

export const useStore = create<State>((set, get) => {
  /** Opens one of the backend's dialogs; what was chosen, or null if it failed (and says why). */
  async function ask<T>(dialog: () => Promise<T>): Promise<T | null> {
    try {
      return await dialog();
    } catch (error) {
      get().toast({ text: String(error), tone: "error" });
      return null;
    }
  }

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
   * Changes the edits of many photos, with progress and a way to stop.
   * Returns the photos that were done and their new versions.
   */
  const applyToMany = async (activity: string, ids: number[], run: () => Promise<AppliedEdits>) => {
    const id = beginActivity(activity, ids.length, () => void api.cancelEdits());
    try {
      // Waits its turn behind anything the editor is still saving.
      const result = await inTurn(() => {
        editsActivity = id;
        return run();
      });
      const done = ids.slice(0, result.versions.length);
      if (result.failed.length > 0) {
        const which = result.failed.length === 1 ? "one thumbnail" : `${result.failed.length} thumbnails`;
        get().toast({ text: `Couldn’t redraw ${which}: ${result.failed[0]}`, tone: "error" });
      }
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
  /** With `settled`, the editor is moving on: the backend first finds the subject and sky in the crop as it now is. */
  const redrawNow = async (settled = false) => {
    clearTimeout(redrawTimer);
    const id = needsRedraw;
    if (id === null) return;
    needsRedraw = null;
    try {
      const [version] = await api.refreshRendered([id], settled);
      patch([id], (p) => ({ ...p, version }));
    } catch (error) {
      get().toast({ text: `Couldn’t update the thumbnail: ${error}`, tone: "error" });
    }
  };
  const redrawSoon = (id: number) => {
    needsRedraw = id;
    clearTimeout(redrawTimer);
    redrawTimer = setTimeout(() => void inTurn(() => redrawNow()), 700);
  };

  // The framing (`framingKey`) the open photo's subject and sky were last
  // found in, and the one they are being found in, so a new crop has them
  // found again once (`reframe`).
  let framedAs: string | null = null;
  let reframing: string | null = null;
  const toReframe = (): Shape[] => {
    const { ready, adjustments } = get().editor;
    return ready ? partsToReframe(adjustments, framedAs) : [];
  };

  const setEditor = (change: Partial<EditorState>) => set((s) => ({ editor: { ...s.editor, ...change } }));

  /**
   * Runs `then` straight away, or when it needs found parts (the subject,
   * the sky, a circled object), once the backend has found them in the open
   * photo: a second or two each the first time. Asked while something else
   * is being found, it waits its turn.
   */
  let findQueue: Promise<unknown> = Promise.resolve();
  let findsWaiting = 0;
  const foundFirst = (shapes: Shape[], then: () => void): Promise<void> | void => {
    if (shapes.length === 0) return then();
    const { photoId, ready, finding } = get().editor;
    if (!ready || photoId === null) return;
    const kinds = new Set(shapes.map((shape) => shape.kind));
    const what = kinds.size > 1 ? "the masks’ parts" : kinds.has("sky") ? "the sky" : kinds.has("object") ? "the object" : "the subject";
    const here = () => get().editor.photoId === photoId;
    findsWaiting++;
    if (finding === null) setEditor({ finding: what });
    const run = async () => {
      try {
        // Opening another photo meanwhile drops what was asked of this one.
        if (!here()) return;
        setEditor({ finding: what });
        // Found in the crop as it is now; the photo's subject and sky are drawn from that crop afterwards.
        const { adjustments } = get().editor;
        await during(`Finding ${what}`, () => api.findParts(photoId, shapes, adjustments));
        if (!here()) return;
        framedAs = framingKey(adjustments);
        setEditor({ found: get().editor.found + 1 });
        then();
      } catch (error) {
        get().toast({ text: `Couldn’t find ${what}: ${error}`, tone: "error" });
      } finally {
        findsWaiting--;
        // Opening another photo meanwhile has already reset the editor.
        if (here() && findsWaiting === 0) setEditor({ finding: null });
      }
    };
    const result = findQueue.then(run);
    findQueue = result;
    return result;
  };

  /** Takes in a history the library just returned for the open photo. */
  const noteHistory = (photoId: number, history: History, checkOut: boolean) => {
    if (get().editor.photoId !== photoId) return;
    // After a jump or a branch switch the sliders follow; after recording a
    // step they are left alone, since another drag may already be under way.
    if (checkOut) {
      // The preset applied last stays adjustable only on the steps it was
      // adjustable on, at the amount it had there.
      const { applied } = get().editor;
      const amount = applied?.steps.get(history.headId);
      setEditor({
        history,
        adjustments: history.adjustments,
        committed: history.adjustments,
        applied:
          applied && amount !== undefined
            ? { ...applied, amount, values: blend(applied.base, applied.preset.settings, amount / 100) }
            : null,
      });
    } else {
      setEditor({ history });
    }
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
    // A crop the subject and sky haven't been found in yet: the thumbnail is drawn once they are.
    if (toReframe().length > 0) needsRedraw = get().editor.photoId;
    if (needsRedraw !== null) void inTurn(() => redrawNow(true));
  };

  const patch = (ids: number[], change: (photo: Photo) => Photo) => {
    const wanted = new Set(ids);
    set((s) => ({ photos: s.photos.map((p) => (wanted.has(p.id) ? change(p) : p)) }));
  };

  /** Drops photos from the current view, keeping selection and viewer sensible. */
  const removeFromView = (ids: number[]) => {
    const gone = new Set(ids);
    const state = get();
    const visible = visiblePhotos(state).map((p) => p.id);
    set({ photos: state.photos.filter((p) => !gone.has(p.id)), ...forget(visible, gone, state) });
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
    presets: [],
    favoritePresets: [],
    cropShape: "Free",
    cropAspect: null,
    brush: { size: 30, feather: 50, strength: 100, erase: false },
    volumes: [],
    importState: null,
    exportState: null,
    filmSheet: null,
    filmSaves: 0,
    toasts: [],
    activities: [],
    confirmRequest: null,
    renamingAlbum: null,
    menu: null,
    shortcutsOpen: false,
    libraryProblem: null,

    async init() {
      const libraryProblem = await api.libraryProblem();
      set({ libraryProblem });
      if (libraryProblem) return;
      const failures = await settleEach([
        () => get().reload(),
        async () => set({ volumes: await api.listVolumes() }),
        async () => set({ presets: await api.listPresets() }),
        async () => set({ favoritePresets: await api.favoritePresets() }),
      ]);
      for (const error of failures) get().toast({ text: String(error), tone: "error" });
    },

    subscribe: () =>
      listenAll({
        "volumes-changed": (payload: Volume[]) => {
          const known = new Set(get().volumes.map((v) => v.path));
          set({ volumes: payload });
          if (get().libraryProblem) return;
          for (const card of payload.filter((v) => !known.has(v.path))) {
            get().toast({
              text: `Camera card “${card.name}” connected`,
              action: { label: "Review photos", run: () => void get().startImport([card.path], card.name) },
            });
          }
        },
        "scan-progress": (payload: Progress) => {
          if (get().importState?.phase === "scanning") set({ importState: { phase: "scanning", progress: payload } });
        },
        "import-progress": (payload: Progress) => {
          const current = get().importState;
          if (current?.phase === "importing") set({ importState: { ...current, progress: payload } });
        },
        "edits-progress": (payload: Progress) => {
          if (editsActivity !== null) advanceActivity(editsActivity, payload.done);
        },
        "export-progress": (payload: Progress) => {
          if (get().exportState?.phase === "exporting") set({ exportState: { phase: "exporting", progress: payload } });
        },
      }),

    async retryLibrary() {
      try {
        await api.retryLibrary();
      } catch {
        set({ libraryProblem: await api.libraryProblem() });
        return;
      }
      await get().init();
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
    setBrush: (change) => set((s) => ({ brush: { ...s.brush, ...change } })),

    setMasks(masks, label) {
      if (!get().editor.ready) return;
      get().adjust({ masks });
      if (label) get().commitAdjust(label);
    },

    addMask(start, drawn) {
      const kind = startKind(start);
      if (kind === "object" && !drawn) return get().startCircle({ mode: null });
      const { size, adjustments } = get().editor;
      if (!size) return;
      const shape = drawn ?? newShape(kind, size, adjustments);
      return foundFirst(isFound(shape) ? [shape] : [], () => {
        const { ready, size, adjustments } = get().editor;
        const { masks } = adjustments;
        if (!ready || !size || masks.length >= MAX_MASKS || !canAdd(masks, kind)) return;
        const mask = newMask(masks, start, size, adjustments, shape);
        setEditor({ maskId: mask.id, partIndex: 0 });
        const name = start === "background" ? "background" : SHAPE_NAMES[kind].toLowerCase();
        get().setMasks([...masks, mask], `New mask: ${name}`);
      });
    },

    addMaskPart(kind, mode, drawn) {
      if (kind === "object" && !drawn) return get().startCircle({ mode });
      const { size, adjustments, maskId: target } = get().editor;
      if (!size || target === null) return;
      const shape = drawn ?? newShape(kind, size, adjustments);
      const name = SHAPE_NAMES[kind].toLowerCase();
      // The part goes to the mask it was asked for, whichever is chosen by the time it's found.
      return foundFirst(isFound(shape) ? [shape] : [], () => {
        const { ready, adjustments, maskId } = get().editor;
        if (!ready) return;
        const mask = adjustments.masks.find((m) => m.id === target);
        if (!mask) return get().toast({ text: `The mask was deleted before the ${name} was found` });
        if (!canAdd(adjustments.masks, kind)) return;
        const parts = [...mask.parts, { mode, shape }];
        if (maskId === target) setEditor({ partIndex: parts.length - 1 });
        get().updateMask(mask.id, { parts }, `${mask.name}: ${mode} ${name}`);
      });
    },

    startCircle(circling) {
      const { ready, photoId } = get().editor;
      if (!ready || photoId === null) return;
      setEditor({ circling });
      // The object model's first look at a photo takes a few seconds: start it while the circle is drawn.
      api.prepareCircles(photoId).catch(() => {});
    },

    cancelCircle: () => setEditor({ circling: null }),

    finishCircle(points) {
      const { circling, adjustments, maskId } = get().editor;
      if (!circling || circling.points) return;
      const shape = circleShape(points);
      const loop = { ...circling, points };
      setEditor({ circling: loop });
      let asked: Promise<void> | void;
      if (circling.replace !== undefined) {
        const mask = adjustments.masks.find((m) => m.id === maskId);
        const part = mask?.parts[circling.replace];
        if (!mask || !part) return setEditor({ circling: null });
        const index = circling.replace;
        // Only the circle changes, on the mask and part it was drawn for, as they are once it's found.
        asked = foundFirst([shape], () => {
          const now = get().editor.adjustments.masks.find((m) => m.id === mask.id);
          const same = now?.parts[index];
          if (!now || !same || JSON.stringify(same.shape) !== JSON.stringify(part.shape)) {
            return get().toast({ text: "The part was removed before the object was found" });
          }
          const parts = now.parts.map((p, i) => (i === index ? { ...p, shape } : p));
          get().updateMask(now.id, { parts }, `${now.name}: circle again`);
        });
      } else if (circling.mode === null) asked = get().addMask("object", shape);
      else asked = get().addMaskPart("object", circling.mode, shape);
      // The loop stays on the photo until its object is found, even behind another find.
      void Promise.resolve(asked).finally(() => {
        if (get().editor.circling === loop) setEditor({ circling: null });
      });
    },

    updateMask(id, change, label) {
      const { masks } = get().editor.adjustments;
      get().setMasks(
        masks.map((mask) => (mask.id === id ? { ...mask, ...change } : mask)),
        label,
      );
    },

    updateMaskPart(index, part, label) {
      const { adjustments, maskId } = get().editor;
      const mask = adjustments.masks.find((m) => m.id === maskId);
      if (!mask || !mask.parts[index]) return;
      get().updateMask(mask.id, { parts: mask.parts.map((p, i) => (i === index ? part : p)) }, label);
    },

    removeMask(id) {
      const { masks } = get().editor.adjustments;
      const mask = masks.find((m) => m.id === id);
      if (!mask) return;
      if (get().editor.maskId === id) setEditor({ maskId: null, partIndex: null });
      get().setMasks(
        masks.filter((m) => m.id !== id),
        `Delete ${mask.name}`,
      );
    },

    removeMaskPart(index) {
      const { adjustments, maskId, partIndex } = get().editor;
      const mask = adjustments.masks.find((m) => m.id === maskId);
      const part = mask?.parts[index];
      if (!mask || !part) return;
      // A mask with nothing left in it goes too.
      if (mask.parts.length === 1) return get().removeMask(mask.id);
      const parts = mask.parts.filter((_, i) => i !== index);
      setEditor({ partIndex: partIndex === null ? null : Math.min(parts.length - 1, partIndex > index ? partIndex - 1 : partIndex) });
      get().updateMask(mask.id, { parts }, `${mask.name}: remove ${SHAPE_NAMES[part.shape.kind].toLowerCase()}`);
    },

    selectMask: (maskId, partIndex = 0) => setEditor({ maskId, partIndex: maskId === null ? null : partIndex }),
    toggleMaskOverlay: () => setEditor({ showMask: !get().editor.showMask }),

    loadEditor(id) {
      settle();
      return inTurn(async () => {
        if (get().openId !== id) return;
        const { showClipping, showMask } = get().editor;
        // A crop shape chosen for one photo doesn't carry over to the next.
        set({ editor: { ...idleEditor, photoId: id, showClipping, showMask }, cropShape: "Free", cropAspect: null });
        try {
          const photo = await api.openEditor(id);
          if (get().openId !== id) return;
          // Opening found the subject and sky in the photo's crop.
          framedAs = framingKey(photo.history.adjustments);
          reframing = null;
          setEditor({
            ready: true,
            size: { width: photo.width, height: photo.height },
            history: photo.history,
            adjustments: photo.history.adjustments,
            committed: photo.history.adjustments,
          });
          for (const what of new Set(photo.missing)) {
            get().toast({ text: `Couldn’t find ${what} in this photo, so that part of its mask is empty`, tone: "error" });
          }
        } catch (error) {
          // Said on the stage, where the photo would be, rather than in a toast.
          if (get().openId === id) setEditor({ failed: String(error) });
        }
      });
    },

    reframe() {
      const { photoId, adjustments } = get().editor;
      const shapes = toReframe();
      const framing = framingKey(adjustments);
      if (photoId === null || shapes.length === 0 || framing === reframing) return;
      reframing = framing;
      // The thumbnail is drawn again once they are found; leaving first finds them before drawing it (`settle`).
      needsRedraw = photoId;
      void foundFirst(shapes, () => redrawSoon(photoId));
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
      const applied = adjustablePreset(get());
      setEditor({ committed: adjustments });
      void inTurn(async () => {
        try {
          const history = await api.historyCommit(photoId, adjustments, name);
          applied?.steps.set(history.headId, applied.amount);
          noteHistory(photoId, history, false);
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
        // crop is left alone: it belongs to this photo's framing. Masks of
        // the subject wait for this photo's to be found, so the edits land whole.
        return foundFirst(foundShapes(clipboard.masks), () => {
          const own = Object.fromEntries(GEOMETRY.map((key) => [key, get().editor.adjustments[key]]));
          get().adjust({ ...clipboard, ...own });
          get().commitAdjust("Paste edits");
        });
      }
      try {
        const { done, versionOf, cancelled } = await applyToMany("Pasting edits", ids, () =>
          api.applyEdits(ids, clipboard, "Paste edits", true),
        );
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
        const { done, versionOf, cancelled } = await applyToMany("Reverting", edited, () =>
          api.applyEdits(edited, DEFAULTS, "Revert to original", false),
        );
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

    setFilm(change, label) {
      const { film } = get().editor.adjustments;
      get().adjust({ film: { ...film, ...change } });
      get().commitAdjust(label);
    },

    setPickingBase: (pickingBase) => setEditor({ pickingBase }),

    async pickBase([x, y]) {
      const { photoId } = get().editor;
      setEditor({ pickingBase: false });
      if (photoId === null) return;
      try {
        const { base } = await api.pickFilmBase(photoId, x, y);
        // A base picked by hand belongs to this frame: its balance is measured afresh.
        if (get().editor.photoId === photoId) get().setFilm({ base, range: null }, "Film base picked");
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
      }
    },

    async balanceRoll(ids) {
      if (ids.length === 0) return;
      try {
        const { done, versionOf, cancelled } = await applyToMany("Balancing the roll", ids, () => api.balanceRoll(ids));
        patch(done, (p) => ({ ...p, version: versionOf.get(p.id) ?? p.version, edited: true, branches: Math.max(1, p.branches) }));
        get().toast({
          text: cancelled
            ? `Stopped: balanced ${done.length} of ${plural(ids.length, "photo")}`
            : `Balanced ${plural(ids.length, "photo")} as one roll`,
        });
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
      }
    },

    previewPreset(preset) {
      if (get().editor.ready) setEditor({ preview: preset });
    },

    async applyPreset(ids, preset) {
      const { editor } = get();
      if (ids.length === 0) return;
      if (ids.length === 1 && ids[0] === editor.photoId && editor.ready) {
        // In the editor it is an ordinary step, and its amount stays adjustable.
        get().commitAdjust();
        const own = get().editor.adjustments;
        const standing = adjustablePreset(get());
        // A preset taking over from the one just applied (the same one again,
        // or another look) runs from the photo as it was before that one, so
        // its amount never builds on the look it replaces.
        const base = standing ? { ...own, ...blend(standing.base, standing.preset.settings, 0) } : own;
        const values = blend(base, preset.settings);
        const steps = standing?.preset.id === preset.id ? standing.steps : new Map<number, number>();
        // A photo that already looked like this has nothing to turn up or down from.
        const applied = holds(base, preset.settings) ? null : { preset, base, amount: 100, values, steps };
        setEditor({ applied, preview: null, adjustments: { ...own, ...values } });
        get().commitAdjust(presetStepLabel(preset, 100));
        return;
      }
      try {
        const { done, versionOf, cancelled } = await applyToMany("Applying preset", ids, () => api.applyPreset(ids, preset.id));
        patch(done, (p) => ({
          ...p,
          version: versionOf.get(p.id) ?? p.version,
          // Close enough until the next reload.
          edited: p.edited || !holds(DEFAULTS, preset.settings),
          branches: Math.max(1, p.branches),
        }));
        get().toast({
          text: cancelled
            ? `Stopped: ${preset.name} applied to ${done.length} of ${plural(ids.length, "photo")}`
            : `${preset.name} applied to ${plural(ids.length, "photo")}`,
        });
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
        await get().reload();
      }
    },

    setPresetAmount(amount) {
      const applied = adjustablePreset(get());
      if (!applied || !get().editor.ready) return;
      const values = blend(applied.base, applied.preset.settings, amount / 100);
      setEditor({ applied: { ...applied, amount, values }, adjustments: { ...get().editor.adjustments, ...values } });
    },

    async savePreset(name, sections) {
      const { editor } = get();
      if (!editor.ready) return;
      const settings = settingsFrom(editor.adjustments, sections);
      if (await attempt(() => api.createPreset(name, settings))) set({ presets: await api.listPresets() });
    },

    async renamePreset(id, name) {
      const current = get().presets.find((preset) => preset.id === id);
      if (!name.trim() || name.trim() === current?.name) return;
      if (await attempt(() => api.renamePreset(id, name))) set({ presets: await api.listPresets() });
    },

    async updatePreset(preset) {
      const { editor } = get();
      if (!editor.ready) return;
      const settings = Object.fromEntries(Object.keys(preset.settings).map((key) => [key, editor.adjustments[key as keyof Adjustments]]));
      if (!(await attempt(() => api.updatePreset(preset.id, settings)))) return;
      set({ presets: await api.listPresets() });
      get().toast({ text: `“${preset.name}” now holds this photo’s edits` });
    },

    async deletePreset(preset) {
      const confirmed = await get().confirm({
        title: `Delete the preset “${preset.name}”?`,
        body: "Photos it was applied to keep their edits.",
        confirmLabel: "Delete preset",
      });
      if (confirmed && (await attempt(() => api.deletePreset(preset.id)))) {
        set({ presets: await api.listPresets(), favoritePresets: await api.favoritePresets() });
      }
    },

    async toggleFavoritePreset(id) {
      const favorite = !get().favoritePresets.includes(id);
      try {
        set({ favoritePresets: await api.setPresetFavorite(id, favorite) });
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
      }
    },

    async importPresets() {
      try {
        const picked = await api.choosePresetFiles();
        if (picked.length === 0) return;
        const { imported, failed } = await api.importPresets(picked);
        set({ presets: await api.listPresets() });
        if (imported.length > 0) get().toast({ text: `Imported ${plural(imported.length, "preset")}` });
        for (const file of failed) get().toast({ text: `Couldn’t import ${file.fileName}: ${file.reason}`, tone: "error" });
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
      }
    },

    async exportPreset(preset) {
      try {
        const path = await api.choosePresetDestination(preset.name.replace(/[/\\:*?"<>|]/g, "-"));
        if (!path) return;
        await api.exportPreset(preset.id, path);
        get().toast({ text: `Saved “${preset.name}” as a file`, action: { label: "Show", run: () => void get().reveal(path) } });
      } catch (error) {
        get().toast({ text: `Couldn’t export the preset: ${error}`, tone: "error" });
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
      const before = visiblePhotos(get()).map((p) => p.id);
      patch(ids, (p) => ({ ...p, flag: next }));
      // Photos the filter now hides leave the selection, as if removed.
      const shown = new Set(visiblePhotos(get()).map((p) => p.id));
      const hidden = new Set(before.filter((id) => !shown.has(id)));
      if (hidden.size > 0) set((s) => forget(before, hidden, s));
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
      const picked = await ask(() => api.chooseImport(false));
      if (picked?.length) await get().startImport(picked);
    },

    async importFolder() {
      const picked = await ask(() => api.chooseImport(true));
      if (picked?.length) await get().startImport(picked);
    },

    async startImport(paths, source) {
      // A drop or a card's "Review photos" while exporting would open a second sheet over the first.
      if (get().exportState) {
        get().toast({ text: "Finish exporting before importing" });
        return;
      }
      if (!canStartImport(get())) return;
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
      if (ids.length === 0 || !canStartExport(get())) return;
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
      const folder = await ask(() => api.chooseExportFolder(current.plan.folder));
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

    async editFilmDetails(ids) {
      if (ids.length === 0 || !canStartFilm(get())) return;
      set({ menu: null });
      try {
        const [photos, suggestions] = await Promise.all([api.getFilmDetails(ids), api.filmSuggestions()]);
        if (photos.length > 0 && canStartFilm(get())) set({ filmSheet: { photos, suggestions } });
      } catch (error) {
        get().toast({ text: String(error), tone: "error" });
      }
    },

    async saveFilmDetails(photos) {
      if (!(await attempt(() => api.setFilmDetails(photos)))) return;
      set((s) => ({ filmSheet: null, filmSaves: s.filmSaves + 1 }));
      if (photos.length > 1) get().toast({ text: `Saved film details for ${plural(photos.length, "photo")}` });
    },

    closeFilmDetails() {
      set({ filmSheet: null });
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
    setShortcutsOpen: (shortcutsOpen) => set({ shortcutsOpen }),
  };
});
