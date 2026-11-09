// The image currently open in Edit. Module-level so it survives switching to Library and back.
import { api } from "$lib/api";
import type { Checkout } from "$lib/bindings/Checkout";
import type { Crop } from "$lib/bindings/Crop";
import type { EditState } from "$lib/bindings/EditState";
import type { HistoryView } from "$lib/bindings/HistoryView";
import type { ImageInfo } from "$lib/bindings/ImageInfo";
import type { Panel } from "$lib/bindings/Panel";
import { scheduleLibraryThumb } from "./versions";

class EditSession {
  image = $state<ImageInfo | null>(null);
  edit = $state<EditState | null>(null);
  history = $state<HistoryView | null>(null);
  loading = $state<string | null>(null);
  error = $state<string | null>(null);
  /** Side-by-side compare with another branch (its tip). */
  compare = $state<{ branch: string; state: EditState } | null>(null);
  /** Branch whose settings the "Copy settings" dialog is offering. */
  applying = $state<string | null>(null);
  /** Crop mode: the crop as it was on entering (for Cancel). Changes commit on Done. */
  cropEntry = $state<Crop | null>(null);
  /** Locked crop aspect (pixel w/h), or null for free. */
  cropAspect = $state<number | null>(null);

  get cropping() {
    return this.cropEntry !== null;
  }

  startCrop() {
    if (!this.edit || this.cropping) return;
    this.compare = null;
    this.cropEntry = $state.snapshot(this.edit.crop);
  }

  finishCrop() {
    if (!this.cropping) return;
    this.cropEntry = null;
    return this.commit(null);
  }

  cancelCrop() {
    if (!this.cropEntry || !this.edit) return;
    this.edit.crop = this.cropEntry;
    this.cropEntry = null;
  }
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

  /** Open a library photo. */
  async open(id: number) {
    this.loading = "Developing…";
    this.error = null;
    try {
      const t0 = performance.now();
      const [info, checkout] = await Promise.all([api.openImage(id), api.history.open(id)]);
      const pixels = await api.previewPixels(info.id);
      console.info(`[tonality] opened ${info.name} ${info.width}×${info.height} in ${Math.round(performance.now() - t0)}ms`);
      this.pixels = pixels;
      this.compare = null;
      this.applying = null;
      this.cropEntry = null;
      this.edit = checkout.state;
      this.history = checkout.view;
      this.image = info;
      scheduleLibraryThumb(id, checkout.state);
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
        const moved = this.history.head.commit !== r.view.head.commit;
        this.history = r.view;
        if (applyState) this.edit = r.state;
        if (moved) scheduleLibraryThumb(id, r.state);
        const cmp = this.compare;
        if (cmp && (!r.view.branches.some((b) => b.name === cmp.branch) || cmp.branch === r.view.head.branch)) {
          this.compare = null;
        }
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
  renameBranch = (old: string, name: string) => {
    if (this.compare?.branch === old) this.compare.branch = name.trim();
    return this.run((id) => api.history.rename(id, old, name), true);
  };
  deleteBranch = (name: string) => this.run((id) => api.history.deleteBranch(id, name), true);
  applyFrom = (source: string, panels: Panel[]) => this.run((id) => api.history.applyFrom(id, source, panels), true);

  /** Show `branch` next to the current version. */
  async startCompare(branch: string) {
    const b = this.history?.branches.find((x) => x.name === branch);
    if (!b || branch === this.history?.head.branch) return;
    try {
      this.compare = { branch, state: await api.history.state(b.tip) };
    } catch (e) {
      this.flashError(String(e));
    }
  }

  stopCompare() {
    this.compare = null;
  }
}

export const session = new EditSession();
