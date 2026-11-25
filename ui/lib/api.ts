// Typed wrappers around Tauri commands. Types come from Rust via ts-rs.
import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Change } from "$lib/bindings/Change";
import type { Checkout } from "$lib/bindings/Checkout";
import type { WbPresetInfo } from "$lib/bindings/WbPresetInfo";
import type { ImageQuery } from "$lib/bindings/ImageQuery";
import type { ImageRow } from "$lib/bindings/ImageRow";
import type { ImportSummary } from "$lib/bindings/ImportSummary";
import type { LibraryOverview } from "$lib/bindings/LibraryOverview";
import type { EditState } from "$lib/bindings/EditState";
import type { Light } from "$lib/bindings/Light";
import type { Panel } from "$lib/bindings/Panel";
import type { GlslProgram } from "$lib/bindings/GlslProgram";
import type { ImageInfo } from "$lib/bindings/ImageInfo";
import type { ExportSettings } from "$lib/bindings/ExportSettings";
import type { ExportSummary } from "$lib/bindings/ExportSummary";
import type { AiTarget } from "$lib/bindings/AiTarget";
import type { AiModelInfo } from "$lib/bindings/AiModelInfo";
import type { LensCorrection } from "$lib/bindings/LensCorrection";
import type { RemoveSummary } from "$lib/bindings/RemoveSummary";
import type { Settings } from "$lib/bindings/Settings";
import type { UpdateInfo } from "$lib/bindings/UpdateInfo";

/**
 * Commands run concurrently, so catalog writes that must land in the order they were made
 * (rating twice quickly, album changes) go through one queue.
 */
let writes: Promise<unknown> = Promise.resolve();
function inOrder<T>(f: () => Promise<T>): Promise<T> {
  const next = writes.then(f, f);
  writes = next.catch(() => {});
  return next;
}

export const api = {
  defaultEditState: () => invoke<EditState>("default_edit_state"),
  wbPresets: () => invoke<WbPresetInfo[]>("wb_presets"),
  /** Auto Light for the open photo, from its preview inside the crop, with `edit`'s white balance. */
  autoTone: (id: number, edit: EditState) => invoke<Light>("auto_tone", { id, edit }),
  /** Auto white balance for the open photo: `[temperature, tint]`, or null if it can't tell. */
  autoWhiteBalance: (id: number, edit: EditState) =>
    invoke<[number, number] | null>("auto_white_balance", { id, edit }),
  startupPath: () => invoke<string | null>("startup_path"),
  startupImport: () => invoke<string | null>("startup_import"),
  devSnapshotPath: () => invoke<string | null>("dev_snapshot_path"),
  devScript: () => invoke<string | null>("dev_script"),
  devZoom: () => invoke<number | null>("dev_zoom"),
  devSaveSnapshot: (dataUrl: string, name: string | null = null) =>
    invoke<void>("dev_save_snapshot", { dataUrl, name }),
  previewShaders: () => invoke<GlslProgram>("preview_shaders"),
  openImage: (id: number) => invoke<ImageInfo>("open_image", { id }),
  previewPixels: async (id: number) =>
    new Uint16Array(await invoke<ArrayBuffer>("preview_pixels", { id })),
  developParams: async (id: number, edit: EditState) =>
    new Float32Array(await invoke<number[]>("develop_params", { id, edit })),
  /** Full-resolution pixels for a region (see `detail_region` in Rust). */
  detailRegion: async (
    id: number,
    r: { x: number; y: number; w: number; h: number; level: number },
  ) => {
    const buf = await invoke<ArrayBuffer>("detail_region", { id, ...r });
    const [x, y, w, h, width, height] = new Uint32Array(buf, 0, 6);
    return { region: { x, y, w, h }, width, height, pixels: new Uint16Array(buf, 24) };
  },
  /** Clarity / Dehaze maps for an open photo: RGBA half floats, `width × height`. */
  presenceMaps: async (id: number) => {
    const buf = await invoke<ArrayBuffer>("presence_maps", { id });
    const [width, height] = new Uint32Array(buf, 0, 2);
    return { width, height, data: new Uint16Array(buf, 8) };
  },
  exportImages: (ids: number[], settings: ExportSettings) =>
    invoke<ExportSummary>("export_images", { ids, settings }),
  /** Stop the running export once the file it's on is written. */
  exportCancel: () => invoke<void>("export_cancel"),
  /**
   * Raster mask layers: `layers` 8-bit `width × height` images, one per brush or AI mask.
   * `missing`: indices of AI masks whose raster hasn't been generated yet.
   */
  maskLayers: async (id: number, edit: EditState) => {
    const buf = await invoke<ArrayBuffer>("mask_layers", { id, edit });
    const [width, height, layers, nMissing] = new Uint32Array(buf, 0, 4);
    const missing = Array.from(new Uint32Array(buf, 16, nMissing));
    return { width, height, layers, missing, data: new Uint8Array(buf, 16 + 4 * nMissing) };
  },
  /** Tone-curve LUT as RGBA half floats (1024 × 1). */
  developCurve: async (edit: EditState) =>
    new Uint16Array(await invoke<ArrayBuffer>("develop_curve", { edit })),

  history: {
    open: (imageId: number) => invoke<Checkout>("history_open", { imageId }),
    commit: (imageId: number, state: EditState, key: string | null) =>
      invoke<Checkout>("history_commit", { imageId, state, key }),
    undo: (imageId: number) => invoke<Checkout>("history_undo", { imageId }),
    redo: (imageId: number) => invoke<Checkout>("history_redo", { imageId }),
    checkout: (imageId: number, commit: number) =>
      invoke<Checkout>("history_checkout", { imageId, commit }),
    switchBranch: (imageId: number, name: string) =>
      invoke<Checkout>("history_switch", { imageId, name }),
    branch: (imageId: number, at: number, name: string | null = null) =>
      invoke<Checkout>("history_branch", { imageId, at, name }),
    rename: (imageId: number, old: string, name: string) =>
      invoke<Checkout>("history_rename", { imageId, old, new: name }),
    deleteBranch: (imageId: number, name: string) =>
      invoke<Checkout>("history_delete", { imageId, name }),
    applyFrom: (imageId: number, source: string, panels: Panel[]) =>
      invoke<Checkout>("history_apply_from", { imageId, source, panels }),
    state: (commit: number) => invoke<EditState>("history_state", { commit }),
    /** What a step changed from the one before it, setting by setting. */
    changes: (commit: number) => invoke<Change[]>("history_changes", { commit }),
    /** Back to as-shot, deleting every version and step. Can't be undone. */
    reset: (imageIds: number[]) => invoke<void>("history_reset", { imageIds }),
  },

  library: {
    overview: () => invoke<LibraryOverview>("library_overview"),
    import: (paths: string[]) => invoke<ImportSummary>("library_import", { paths }),
    /** Stop the running import: photos already copied stay, the rest are left out. */
    cancelImport: () => invoke<void>("library_import_cancel"),
    /** Catalogue originals in the library folder the catalog doesn't know (restoring sidecar edits). */
    recover: () => invoke<ImportSummary>("library_recover"),
    images: (query: ImageQuery) => invoke<ImageRow[]>("library_images", { query }),
    /** Take photos out of the library; their files go to the system's Trash / Recycle Bin. */
    remove: (ids: number[]) => inOrder(() => invoke<RemoveSummary>("library_remove", { ids })),
    setRating: (ids: number[], rating: number) =>
      inOrder(() => invoke<void>("library_set_rating", { ids, rating })),
    setFlag: (ids: number[], flag: number) =>
      inOrder(() => invoke<void>("library_set_flag", { ids, flag })),
  },

  ai: {
    models: () => invoke<AiModelInfo[]>("ai_models"),
    modelFor: (target: AiTarget) => invoke<string>("ai_model_for", { target }),
    /** Detect `target` (downloading the model if needed); resolves to whether anything was found. */
    detect: (imageId: number, target: AiTarget, model: string, lens: LensCorrection | null) =>
      invoke<boolean>("ai_detect", { imageId, target, model, lens }),
  },

  settings: {
    get: () => invoke<Settings>("settings_get"),
    set: (settings: Settings) => invoke<void>("settings_set", { settings }),
  },

  /** The app's log file, or null if it couldn't be opened. */
  logPath: () => invoke<string | null>("log_path"),

  updates: {
    version: () => invoke<string>("app_version"),
    /** Ask the selected channel for an update; null when up to date. */
    check: () => invoke<UpdateInfo | null>("update_check"),
    /** Download the update the last check found (progress: `update-progress` events). */
    download: () => invoke<void>("update_download"),
    /** Install the downloaded update and restart into it. */
    install: () => invoke<void>("update_install"),
  },

  albums: {
    create: (name: string, images: number[] = []) =>
      inOrder(() => invoke<number>("album_create", { name, images })),
    rename: (id: number, name: string) => inOrder(() => invoke<void>("album_rename", { id, name })),
    delete: (id: number) => inOrder(() => invoke<void>("album_delete", { id })),
    add: (id: number, images: number[]) =>
      inOrder(() => invoke<number>("album_add", { id, images })),
    remove: (id: number, images: number[]) =>
      inOrder(() => invoke<void>("album_remove", { id, images })),
  },
};

/** URL of an image's library thumbnail (served by the `thumb://` protocol). */
export const thumbUrl = (id: number) => convertFileSrc(String(id), "thumb");

export const RAW_EXTENSIONS = [
  "arw",
  "cr2",
  "cr3",
  "crw",
  "nef",
  "nrw",
  "raf",
  "orf",
  "rw2",
  "pef",
  "srw",
  "dng",
  "3fr",
  "iiq",
  "erf",
  "mef",
  "mos",
  "mrw",
  "kdc",
  "dcr",
];
