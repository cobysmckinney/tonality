import { convertFileSrc, invoke } from "@tauri-apps/api/core";
import type { Adjustments, Shape } from "./adjustments";
import type { Preset, PresetSettings } from "./presets";

export type View =
  | { kind: "library" }
  | { kind: "favorites" }
  | { kind: "imports" }
  | { kind: "deleted" }
  | { kind: "album"; id: number };

/** 1 = pick, -1 = reject, 0 = unflagged. */
export type Flag = -1 | 0 | 1;

export interface Photo {
  id: number;
  fileName: string;
  kind: "raw" | "image";
  hasJpeg: boolean;
  /** Camera-local capture time, `YYYY-MM-DDTHH:MM:SS`. */
  takenAt: string;
  width: number | null;
  height: number | null;
  favorite: boolean;
  flag: Flag;
  importId: number;
  /** Unix seconds; set while the photo is in Recently Deleted. */
  deletedAt: number | null;
  edited: boolean;
  /** Changes whenever the photo's rendering does; keeps stale thumbnails out. */
  version: number;
  /** How many branches of edits the photo has; the grid shows the current one. */
  branches: number;
  /** The original is no longer in the library folder: moved or deleted outside the app. */
  missing: boolean;
}

export interface PhotoInfo {
  id: number;
  path: string;
  jpegPath: string | null;
  fileSize: number;
  make: string | null;
  model: string | null;
  lens: string | null;
  iso: number | null;
  aperture: number | null;
  shutter: number | null;
  focalLength: number | null;
  importedAt: number;
  albums: string[];
}

export interface Album {
  id: number;
  name: string;
  count: number;
}

export interface ImportInfo {
  id: number;
  createdAt: number;
  source: string;
}

export interface Overview {
  libraryPath: string;
  photoCount: number;
  favoriteCount: number;
  deletedCount: number;
  albums: Album[];
  imports: ImportInfo[];
}

export interface Volume {
  name: string;
  path: string;
}

export interface ScanItem {
  index: number;
  fileName: string;
  kind: "raw" | "image";
  hasJpeg: boolean;
  size: number;
  /** When it was taken, as near as the scan could tell: `YYYY-MM-DDTHH:MM:SS`, camera-local. */
  takenAt: string;
  /** `deleted` means the photo is in Recently Deleted; importing it recovers it. */
  status: "new" | "duplicate" | "deleted";
}

export interface Scan {
  sessionId: number;
  source: string;
  items: ScanItem[];
}

export interface ImportSummary {
  imported: number;
  restored: number;
  failed: { fileName: string; reason: string }[];
  cancelled: boolean;
}

/** The part of the picture to draw, in 0..1 coordinates of the cropped frame. */
export interface Region {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface Branch {
  id: number;
  name: string;
}

export interface Step {
  id: number;
  /** What changed, in words: "Exposure +0.50". */
  label: string;
  /** Unix seconds. */
  createdAt: number;
  /** Other branches that part ways with the current one right after this step. */
  forks: string[];
  /** The files exported from exactly this step, oldest first. */
  exports: { path: string; createdAt: number }[];
}

/** A photo's edit history as seen from its current branch. See history.rs. */
export interface History {
  branches: Branch[];
  branchId: number;
  /** The current branch from the original to its newest step, oldest first. */
  steps: Step[];
  /** Where you are on the branch; steps after it have been undone. */
  headId: number;
  /** The recipe at the head. */
  adjustments: Adjustments;
}

export interface EditorPhoto {
  /** The photo's full size in pixels, before any rotation or crop. */
  width: number;
  height: number;
  history: History;
  /** Found parts of its masks that couldn't be found in this photo ("the object"); they are left empty. */
  missing: string[];
}

export interface Frame {
  pixels: ImageData;
  /** 256 bins each for red, green, blue and brightness, back to back. */
  histogram: Uint32Array;
  highlightsClipped: boolean;
  shadowsClipped: boolean;
}

/** Unpacks the binary reply of `render_frame`; the layout is documented there. */
function decodeFrame(buffer: ArrayBuffer): Frame {
  const header = new Uint32Array(buffer, 0, 4);
  const [width, height, flags] = header;
  const histogramBytes = 4 * 256 * 4;
  return {
    pixels: new ImageData(new Uint8ClampedArray(buffer, 16 + histogramBytes, width * height * 4), width, height),
    histogram: new Uint32Array(buffer, 16, 4 * 256),
    highlightsClipped: (flags & 1) !== 0,
    shadowsClipped: (flags & 2) !== 0,
  };
}

export interface ExportSettings {
  /** The folder the files are written to; null is the library's own Exports folder. */
  folder: string | null;
  /** TIFF is 16 bits a channel and uncompressed; the others are 8-bit. */
  format: "jpeg" | "png" | "tiff";
  /** JPEG quality, 1 to 100. */
  quality: number;
  /** The longer side of the exported picture in pixels; null keeps it at full size. */
  longEdge: number | null;
}

/** What to export, as opposed to how. See export.rs. */
export interface ExportJob {
  ids: number[];
  /** Each photo's current branch, every branch of every photo, or these branches of a single photo. */
  branches: "current" | "all" | { chosen: number[] };
  /**
   * What to call the files, without the extension; `{name}`, `{branch}`,
   * `{date}` and `{n}` are filled in for each file. Null names each file
   * after its photo, and its branch if it has several.
   */
  name: string | null;
}

/** What an export would write, worked out before anything is written. */
export interface ExportPlan {
  settings: ExportSettings;
  /** The folder the files go to, in full. */
  folder: string;
  files: {
    photoId: number;
    /** The branch the file is drawn from; null for a photo with no history yet. */
    branchId: number | null;
    /** That branch's name, for a photo that has more than one. */
    branch: string | null;
    name: string;
    /** True when the name asked for is taken, so this one carries a number. */
    numbered: boolean;
    /** The picture's size in pixels, where the library knows the photo's. */
    width: number | null;
    height: number | null;
  }[];
  /** For a single photo, its branches to choose between and the one it is on. */
  branches: Branch[];
  currentBranchId: number | null;
  /** True when some photo has more than one branch. */
  severalBranches: boolean;
}

export interface Exported {
  photoId: number;
  name: string;
  path: string;
  /** The branch the file was drawn from, for a photo that has more than one. */
  branch: string | null;
  width: number;
  height: number;
  fileSize: number;
}

export interface ExportSummary {
  folder: string;
  exported: Exported[];
  failed: { fileName: string; reason: string }[];
  cancelled: boolean;
}

export interface AppliedEdits {
  /** The new versions of the photos that were done: the first so many of those asked for, in order. */
  versions: number[];
  /** True if it was stopped before reaching every photo. */
  cancelled: boolean;
  /** Why thumbnails couldn't be redrawn, one per photo it happened to. Their edits were still applied. */
  failed: string[];
}

/** What reading preset files came to. */
export interface ImportedPresets {
  imported: Preset[];
  failed: { fileName: string; reason: string }[];
}

export interface Progress {
  done: number;
  total: number;
}

export const api = {
  getOverview: () => invoke<Overview>("get_overview"),
  listPhotos: (view: View) => invoke<Photo[]>("list_photos", { view }),
  getPhotoInfo: (id: number) => invoke<PhotoInfo>("get_photo_info", { id }),

  setFavorite: (ids: number[], favorite: boolean) => invoke<void>("set_favorite", { ids, favorite }),
  setFlag: (ids: number[], flag: Flag) => invoke<void>("set_flag", { ids, flag }),

  trashPhotos: (ids: number[]) => invoke<void>("trash_photos", { ids }),
  restorePhotos: (ids: number[]) => invoke<void>("restore_photos", { ids }),
  purgePhotos: (ids: number[]) => invoke<number>("purge_photos", { ids }),

  createAlbum: (name: string, ids: number[]) => invoke<number>("create_album", { name, ids }),
  renameAlbum: (id: number, name: string) => invoke<void>("rename_album", { id, name }),
  deleteAlbum: (id: number) => invoke<void>("delete_album", { id }),
  addToAlbum: (albumId: number, ids: number[]) => invoke<void>("add_to_album", { albumId, ids }),
  removeFromAlbum: (albumId: number, ids: number[]) => invoke<void>("remove_from_album", { albumId, ids }),

  listVolumes: () => invoke<Volume[]>("list_volumes"),
  scanImport: (paths: string[], source?: string) => invoke<Scan>("scan_import", { paths, source }),
  discardScan: () => invoke<void>("discard_scan"),
  runImport: (sessionId: number, indices: number[]) => invoke<ImportSummary>("run_import", { sessionId, indices }),
  cancelImport: () => invoke<void>("cancel_import"),

  openEditor: (id: number) => invoke<EditorPhoto>("open_editor", { id }),
  closeEditor: () => invoke<void>("close_editor"),
  renderFrame: (
    id: number,
    adjustments: Adjustments,
    region: Region,
    width: number,
    height: number,
    showClipping: boolean,
    /** Ignore the crop and draw the whole tilted photo: the crop tool's view. */
    uncropped: boolean,
    /** The mask, by id, to tint red where it applies. */
    maskOverlay: number | null,
  ) =>
    invoke<ArrayBuffer>("render_frame", { id, adjustments, region, width, height, showClipping, uncropped, maskOverlay }).then(
      decodeFrame,
    ),
  /** Each mask's coverage as a small black-and-white picture, in the order of `adjustments.masks`. */
  maskMattes: (id: number, adjustments: Adjustments, longEdge: number) =>
    invoke<ArrayBuffer>("mask_mattes", { id, adjustments, longEdge }).then((buffer) => {
      const [width, height, count] = new Uint32Array(buffer, 0, 3);
      return Array.from({ length: count }, (_, i) => {
        const gray = new Uint8Array(buffer, 16 + i * width * height, width * height);
        const pixels = new ImageData(width, height);
        gray.forEach((value, p) => pixels.data.set([value, value, value, 255], p * 4));
        return pixels;
      });
    }),
  /** Finds what these parts pick out of the open photo (subject, sky, circled object), so masks can use them. */
  findParts: (id: number, shapes: Shape[]) => invoke<void>("find_parts", { id, shapes }),
  /** Starts the object model's first look at the open photo, so a circle being drawn is answered quickly. */
  prepareCircles: (id: number) => invoke<void>("prepare_circles", { id }),
  /** Redraws thumbnails to match the photos' current edits; returns each photo's new version. */
  refreshRendered: (ids: number[]) => invoke<number[]>("refresh_rendered", { ids }),
  /**
   * Applies one recipe to these photos as a step in each one's history, one
   * photo at a time, sending an `edits-progress` event after each. With
   * `keepCrop` each photo keeps its own crop, straightening, turns and flips.
   */
  applyEdits: (ids: number[], adjustments: Adjustments, label: string, keepCrop: boolean) =>
    invoke<AppliedEdits>("apply_edits", { ids, adjustments, label, keepCrop }),
  /** Stops `applyEdits` after the photo it is on. */
  cancelEdits: () => invoke<void>("cancel_edits"),
  getEdits: (id: number) => invoke<Adjustments>("get_edits", { id }),

  /** Every preset: the built-in ones in their order, then your own by name. */
  listPresets: () => invoke<Preset[]>("list_presets"),
  createPreset: (name: string, settings: PresetSettings) => invoke<Preset>("create_preset", { name, settings }),
  renamePreset: (id: number, name: string) => invoke<Preset>("rename_preset", { id, name }),
  updatePreset: (id: number, settings: PresetSettings) => invoke<Preset>("update_preset", { id, settings }),
  deletePreset: (id: number) => invoke<void>("delete_preset", { id }),
  /** The starred presets' ids, in the order they were starred. */
  favoritePresets: () => invoke<number[]>("favorite_presets"),
  setPresetFavorite: (id: number, favorite: boolean) => invoke<number[]>("set_preset_favorite", { id, favorite }),
  /** The open photo with each preset laid over these edits, small, by preset id. */
  presetPreviews: (id: number, adjustments: Adjustments, longEdge: number) =>
    invoke<ArrayBuffer>("preset_previews", { id, adjustments, longEdge }).then((buffer) => {
      const [width, height, count] = new Uint32Array(buffer, 0, 3);
      const each = 8 + width * height * 4;
      const previews = new Map<number, ImageData>();
      for (let i = 0; i < count; i++) {
        const at = 16 + i * each;
        const id = Number(new DataView(buffer, at, 8).getBigInt64(0, true));
        previews.set(id, new ImageData(new Uint8ClampedArray(buffer.slice(at + 8, at + each)), width, height));
      }
      return previews;
    }),
  exportPreset: (id: number, path: string) => invoke<void>("export_preset", { id, path }),
  importPresets: (paths: string[]) => invoke<ImportedPresets>("import_presets", { paths }),
  /** Lays a preset over each photo's own edits, the way `applyEdits` applies a recipe. */
  applyPreset: (ids: number[], presetId: number) => invoke<AppliedEdits>("apply_preset", { ids, presetId }),

  getHistory: (id: number) => invoke<History>("get_history", { id }),
  historyCommit: (id: number, adjustments: Adjustments, label: string) =>
    invoke<History>("history_commit", { id, adjustments, label }),
  historyGoto: (id: number, stepId: number) => invoke<History>("history_goto", { id, stepId }),
  historyBranch: (id: number, stepId: number, name: string) => invoke<History>("history_branch", { id, stepId, name }),
  historySwitch: (id: number, branchId: number) => invoke<History>("history_switch", { id, branchId }),
  historyRenameBranch: (id: number, branchId: number, name: string) =>
    invoke<History>("history_rename_branch", { id, branchId, name }),
  historyDeleteBranch: (id: number, branchId: number) => invoke<History>("history_delete_branch", { id, branchId }),

  /** What a job would write. Without `settings`, the ones used last time apply. */
  planExport: (job: ExportJob, settings: ExportSettings | null) => invoke<ExportPlan>("plan_export", { job, settings }),
  runExport: (job: ExportJob, settings: ExportSettings) => invoke<ExportSummary>("run_export", { job, settings }),
  cancelExport: () => invoke<void>("cancel_export"),
};

// `photo://localhost/` on Linux and macOS, `http://photo.localhost/` on Windows.
const IMAGE_BASE = convertFileSrc("", "photo");

type Versioned = Pick<Photo, "id" | "version">;
export const thumbUrl = (photo: Versioned) => `${IMAGE_BASE}thumb/${photo.id}?v=${photo.version}`;
export const previewUrl = (photo: Versioned) => `${IMAGE_BASE}preview/${photo.id}?v=${photo.version}`;
export const scanThumbUrl = (sessionId: number, index: number) => `${IMAGE_BASE}scan/${sessionId}/${index}`;
