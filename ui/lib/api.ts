// Typed wrappers around Tauri commands. Types come from Rust via ts-rs.
import { invoke } from "@tauri-apps/api/core";
import type { EditState } from "$lib/bindings/EditState";
import type { GlslProgram } from "$lib/bindings/GlslProgram";
import type { ImageInfo } from "$lib/bindings/ImageInfo";

export const api = {
  defaultEditState: () => invoke<EditState>("default_edit_state"),
  startupPath: () => invoke<string | null>("startup_path"),
  devSnapshotPath: () => invoke<string | null>("dev_snapshot_path"),
  devSaveSnapshot: (dataUrl: string) => invoke<void>("dev_save_snapshot", { dataUrl }),
  previewShaders: () => invoke<GlslProgram>("preview_shaders"),
  openImage: (path: string) => invoke<ImageInfo>("open_image", { path }),
  previewPixels: async (id: string) =>
    new Uint16Array(await invoke<ArrayBuffer>("preview_pixels", { id })),
  developParams: async (id: string, edit: EditState) =>
    new Float32Array(await invoke<number[]>("develop_params", { id, edit })),
};

export const RAW_EXTENSIONS = [
  "arw", "cr2", "cr3", "crw", "nef", "nrw", "raf", "orf", "rw2", "pef", "srw", "dng", "3fr",
  "iiq", "erf", "mef", "mos", "mrw", "kdc", "dcr", "x3f",
];
