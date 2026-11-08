// Typed wrappers around Tauri commands. Types come from Rust via ts-rs.
import { invoke } from "@tauri-apps/api/core";
import { convertFileSrc } from "@tauri-apps/api/core";
import type { Checkout } from "$lib/bindings/Checkout";
import type { ImageQuery } from "$lib/bindings/ImageQuery";
import type { ImageRow } from "$lib/bindings/ImageRow";
import type { ImportSummary } from "$lib/bindings/ImportSummary";
import type { LibraryOverview } from "$lib/bindings/LibraryOverview";
import type { EditState } from "$lib/bindings/EditState";
import type { GlslProgram } from "$lib/bindings/GlslProgram";
import type { ImageInfo } from "$lib/bindings/ImageInfo";

export const api = {
  defaultEditState: () => invoke<EditState>("default_edit_state"),
  startupPath: () => invoke<string | null>("startup_path"),
  startupImport: () => invoke<string | null>("startup_import"),
  devSnapshotPath: () => invoke<string | null>("dev_snapshot_path"),
  devSaveSnapshot: (dataUrl: string) => invoke<void>("dev_save_snapshot", { dataUrl }),
  previewShaders: () => invoke<GlslProgram>("preview_shaders"),
  openImage: (id: number) => invoke<ImageInfo>("open_image", { id }),
  previewPixels: async (id: number) =>
    new Uint16Array(await invoke<ArrayBuffer>("preview_pixels", { id })),
  developParams: async (id: number, edit: EditState) =>
    new Float32Array(await invoke<number[]>("develop_params", { id, edit })),

  history: {
    open: (imageId: number) => invoke<Checkout>("history_open", { imageId }),
    commit: (imageId: number, state: EditState, key: string | null) =>
      invoke<Checkout>("history_commit", { imageId, state, key }),
    undo: (imageId: number) => invoke<Checkout>("history_undo", { imageId }),
    redo: (imageId: number) => invoke<Checkout>("history_redo", { imageId }),
    checkout: (imageId: number, commit: number) => invoke<Checkout>("history_checkout", { imageId, commit }),
    switchBranch: (imageId: number, name: string) => invoke<Checkout>("history_switch", { imageId, name }),
    branch: (imageId: number, at: number, name: string | null = null) =>
      invoke<Checkout>("history_branch", { imageId, at, name }),
    rename: (imageId: number, old: string, name: string) =>
      invoke<Checkout>("history_rename", { imageId, old, new: name }),
    deleteBranch: (imageId: number, name: string) => invoke<Checkout>("history_delete", { imageId, name }),
  },

  library: {
    overview: () => invoke<LibraryOverview>("library_overview"),
    import: (paths: string[]) => invoke<ImportSummary>("library_import", { paths }),
    images: (query: ImageQuery) => invoke<ImageRow[]>("library_images", { query }),
    setRating: (ids: number[], rating: number) => invoke<void>("library_set_rating", { ids, rating }),
    setFlag: (ids: number[], flag: number) => invoke<void>("library_set_flag", { ids, flag }),
  },
};

/** URL of an image's library thumbnail (served by the `thumb://` protocol). */
export const thumbUrl = (id: number) => convertFileSrc(String(id), "thumb");

export const RAW_EXTENSIONS = [
  "arw", "cr2", "cr3", "crw", "nef", "nrw", "raf", "orf", "rw2", "pef", "srw", "dng", "3fr",
  "iiq", "erf", "mef", "mos", "mrw", "kdc", "dcr", "x3f",
];
