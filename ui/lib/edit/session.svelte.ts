// The image currently open in Edit. Module-level so it survives switching to Library and back.
import { open } from "@tauri-apps/plugin-dialog";
import { api, RAW_EXTENSIONS } from "$lib/api";
import type { EditState } from "$lib/bindings/EditState";
import type { ImageInfo } from "$lib/bindings/ImageInfo";

class EditSession {
  image = $state<ImageInfo | null>(null);
  edit = $state<EditState | null>(null);
  loading = $state<string | null>(null);
  error = $state<string | null>(null);
  /** Preview pixels (not reactive: large and replaced wholesale). Read when `image` changes. */
  pixels: Uint16Array | null = null;
  private defaults: EditState | null = null;

  async defaultEdit(): Promise<EditState> {
    this.defaults ??= await api.defaultEditState();
    return structuredClone(this.defaults);
  }

  async pickAndOpen() {
    const path = await open({
      multiple: false,
      directory: false,
      filters: [{ name: "RAW images", extensions: [...RAW_EXTENSIONS, ...RAW_EXTENSIONS.map((e) => e.toUpperCase())] }],
    });
    if (typeof path === "string") await this.open(path);
  }

  async open(path: string) {
    const name = path.split(/[\\/]/).pop() ?? path;
    this.loading = `Developing ${name}…`;
    this.error = null;
    try {
      const t0 = performance.now();
      const info = await api.openImage(path);
      const pixels = await api.previewPixels(info.id);
      console.info(`[tonality] opened ${info.name} ${info.width}×${info.height} in ${Math.round(performance.now() - t0)}ms`);
      this.pixels = pixels;
      this.edit = await this.defaultEdit();
      this.image = info;
    } catch (e) {
      this.error = String(e);
    } finally {
      this.loading = null;
    }
  }
}

export const session = new EditSession();
