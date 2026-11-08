// The image currently open in Edit. Module-level so it survives switching to Library and back.
import { open } from "@tauri-apps/plugin-dialog";
import { api, RAW_EXTENSIONS } from "$lib/api";
import type { Checkout } from "$lib/bindings/Checkout";
import type { EditState } from "$lib/bindings/EditState";
import type { HistoryView } from "$lib/bindings/HistoryView";
import type { ImageInfo } from "$lib/bindings/ImageInfo";

class EditSession {
  image = $state<ImageInfo | null>(null);
  edit = $state<EditState | null>(null);
  history = $state<HistoryView | null>(null);
  loading = $state<string | null>(null);
  error = $state<string | null>(null);
  /** Preview pixels (not reactive: large and replaced wholesale). Read when `image` changes. */
  pixels: Uint16Array | null = null;
  private defaults: EditState | null = null;
  /** History operations run strictly in order, so a fast undo can't overtake a pending commit. */
  private queue: Promise<unknown> = Promise.resolve();
  private errorTimer: ReturnType<typeof setTimeout> | undefined;

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
      const [info, checkout] = await Promise.all([api.openImage(path), api.history.open(path)]);
      const pixels = await api.previewPixels(info.id);
      console.info(`[tonality] opened ${info.name} ${info.width}×${info.height} in ${Math.round(performance.now() - t0)}ms`);
      this.pixels = pixels;
      this.edit = checkout.state;
      this.history = checkout.view;
      this.image = info;
    } catch (e) {
      this.error = String(e);
    } finally {
      this.loading = null;
    }
  }

  private run(op: (imageId: number) => Promise<Checkout>, applyState: boolean): Promise<void> {
    const next = this.queue.then(async () => {
      const id = this.history?.imageId;
      if (id === undefined) return;
      try {
        const r = await op(id);
        if (this.history?.imageId !== id) return; // another image was opened meanwhile
        this.history = r.view;
        if (applyState) this.edit = r.state;
      } catch (e) {
        this.flashError(String(e));
        throw e;
      }
    });
    this.queue = next.catch(() => {});
    return next;
  }

  private flashError(msg: string) {
    this.error = msg;
    clearTimeout(this.errorTimer);
    this.errorTimer = setTimeout(() => (this.error = null), 4000);
  }

  /** Record the current edit as a commit. `key` names the control, so rapid nudges coalesce. */
  commit(key: string | null) {
    if (!this.edit) return Promise.resolve();
    const state = $state.snapshot(this.edit);
    // Don't overwrite `edit` with the reply: the user may already be dragging again.
    return this.run((id) => api.history.commit(id, state, key), false);
  }

  undo = () => this.run((id) => api.history.undo(id), true);
  redo = () => this.run((id) => api.history.redo(id), true);
  checkout = (commit: number) => this.run((id) => api.history.checkout(id, commit), true);
  switchBranch = (name: string) => this.run((id) => api.history.switchBranch(id, name), true);
  branch = (at: number, name: string | null = null) => this.run((id) => api.history.branch(id, at, name), true);
  renameBranch = (old: string, name: string) => this.run((id) => api.history.rename(id, old, name), true);
  deleteBranch = (name: string) => this.run((id) => api.history.deleteBranch(id, name), true);
}

export const session = new EditSession();
