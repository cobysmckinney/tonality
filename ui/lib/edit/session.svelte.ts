// The image currently open in Edit. Module-level so it survives switching to Library and back.
import { listen } from "@tauri-apps/api/event";
import { ask } from "@tauri-apps/plugin-dialog";
import { SvelteMap } from "svelte/reactivity";
import { api } from "$lib/api";
import type { AiTarget } from "$lib/bindings/AiTarget";
import type { Checkout } from "$lib/bindings/Checkout";
import type { Crop } from "$lib/bindings/Crop";
import type { EditState } from "$lib/bindings/EditState";
import type { HistoryView } from "$lib/bindings/HistoryView";
import type { ImageInfo } from "$lib/bindings/ImageInfo";
import type { Panel } from "$lib/bindings/Panel";
import type { Mask } from "$lib/bindings/Mask";
import { scheduleLibraryThumb } from "./versions";
import { MAX_MASKS, aiKey, masksOf, newAiMask, newMask, type AiPreset, type BrushTool, type MaskKind } from "./masks";

/** Where an AI mask's detection stands (keyed by `aiKey`). */
export type AiStatus =
  | { state: "working" }
  | { state: "found" }
  | { state: "empty" }
  | { state: "error"; message: string };

/** The latest `ai-progress` event: a model download, or a detection running. */
export interface AiProgress {
  /** The photo it's for (null: an export's detection). */
  image: number | null;
  stage: "download" | "detect";
  model: string;
  done: number;
  total: number;
}
import type { Framing } from "./renderer";

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

  /** Mask mode: the mask list replaces the adjustments, and the selected mask has handles. */
  masking = $state(false);
  maskIndex = $state<number | null>(null);
  /** Tint the selected mask's area (O). */
  maskOverlay = $state(false);
  brush = $state<BrushTool>({ size: 0.04, feather: 0.6, strength: 1, erase: false });
  /** Detections, by `aiKey`. */
  aiStatus = new SvelteMap<string, AiStatus>();
  aiProgress = $state<AiProgress | null>(null);
  /** Bumped when a detection finishes, so the raster layers are fetched again. */
  aiEpoch = $state(0);
  /** Drawing a loop for Select Object: a new mask, or re-selecting mask `replace`. */
  lassoing = $state<{ replace: number | null; busy?: number } | null>(null);
  private lassoSeq = 0;
  /** Models the user agreed to download this session. */
  private approved = new Set<string>();

  constructor() {
    listen<AiProgress>("ai-progress", (e) => {
      const p = e.payload;
      if (p.image === null || p.image !== this.image?.id) return; // another photo's, or export's
      this.aiProgress = p.stage === "detect" && p.done >= p.total ? null : p;
    }).catch(() => {});
  }

  /** The main view's framing, for placing new masks (set by Canvas). */
  framing = $state.raw<Framing | null>(null);

  get masks(): Mask[] {
    return this.edit ? masksOf(this.edit) : [];
  }

  /** The selected mask's index, if it (still) exists. */
  get selected(): number | null {
    const i = this.maskIndex;
    return i !== null && i < this.masks.length ? i : null;
  }

  startMasks() {
    if (!this.edit || this.cropping) return;
    this.compare = null;
    this.masking = true;
    if (this.selected === null && this.masks.length) this.maskIndex = this.masks.length - 1;
  }

  stopMasks() {
    this.masking = false;
    this.lassoing = null;
  }

  addMask(kind: MaskKind) {
    const img = this.image;
    if (!this.edit || !img || this.masks.length >= MAX_MASKS) return;
    const fr = this.framing ?? { w: img.fullWidth, h: img.fullHeight, m: [1, 0, 0, 0, 1, 0] };
    const m = newMask(kind, this.masks, fr, img.fullWidth, img.fullHeight);
    this.edit.masks = [...this.masks, m];
    this.maskIndex = this.edit.masks.length - 1;
    return this.commit(null);
  }

  /** Ask before a model's first download. Resolves false if declined. */
  private async confirmModel(model: string): Promise<boolean> {
    if (this.approved.has(model)) return true;
    const info = (await api.ai.models()).find((m) => m.id === model);
    if (!info || info.installed) {
      this.approved.add(model);
      return true;
    }
    const mb = Math.round(info.size / 1e6);
    const ok = await ask(
      `${info.label} masks need a one-time ${mb} MB download. The model runs on this computer: your photos never leave it.`,
      { title: "Download AI model", okLabel: `Download ${mb} MB`, kind: "info" },
    );
    if (ok) this.approved.add(model);
    return ok;
  }

  /** Add an AI mask and detect it. */
  async addAiMask(preset: Exclude<AiPreset, "object">) {
    if (!this.edit || this.masks.length >= MAX_MASKS) return;
    const target: AiTarget = preset === "background" ? { kind: "subject" } : { kind: preset };
    return this.addAi(preset, target);
  }

  private async addAi(preset: AiPreset, target: AiTarget) {
    const model = await api.ai.modelFor(target);
    if (!(await this.confirmModel(model)) || !this.edit) return;
    const m = newAiMask(preset, target, model, this.masks);
    this.edit.masks = [...this.masks, m];
    this.maskIndex = this.edit.masks.length - 1;
    this.commit(null);
    await this.detect(m);
  }

  startLasso(replace: number | null = null) {
    if (!this.edit || (replace === null && this.masks.length >= MAX_MASKS)) return;
    this.lassoing = { replace };
  }

  cancelLasso() {
    this.lassoing = null;
  }

  /** A loop was drawn (source uv): select the object in it. */
  async finishLasso(lasso: [number, number][]) {
    const job = this.lassoing;
    if (!job || job.busy) return;
    if (!this.edit || lasso.length < 3) {
      this.lassoing = null;
      return;
    }
    // The loop stays on screen while the object is detected.
    // (`busy` numbers the detection: `lassoing` is a proxy, so compare by value.)
    const busy = ++this.lassoSeq;
    this.lassoing = { ...job, busy };
    try {
      const target: AiTarget = { kind: "object", lasso };
      const i = job.replace;
      const old = i !== null ? this.edit.masks?.[i] : undefined;
      if (old && old.shape.kind === "ai") {
        old.shape.target = target;
        old.shape.model = await api.ai.modelFor(target);
        this.commit(null);
        await this.detect(old);
        return;
      }
      await this.addAi("object", target);
    } finally {
      if (this.lassoing?.busy === busy) this.lassoing = null;
    }
  }

  /** Run the model for an AI mask, unless it's already running. */
  async detect(m: Mask) {
    const key = aiKey(m);
    const id = this.image?.id;
    if (!key || id === undefined || m.shape.kind !== "ai" || this.aiStatus.get(key)?.state === "working") return;
    const { target, model } = $state.snapshot(m.shape);
    this.aiStatus.set(key, { state: "working" });
    // Statuses are per photo (cleared on open): a result for one we've left is dropped.
    const here = () => this.image?.id === id;
    try {
      const found = await api.ai.detect(id, target, model, this.edit?.lens ?? null);
      if (!here()) return;
      this.aiStatus.set(key, { state: found ? "found" : "empty" });
      // A scene pass also caches its sibling (sky ↔ people).
      if (target.kind === "sky" || target.kind === "people") {
        const other = target.kind === "sky" ? "people" : "sky";
        const k = `${model}:${JSON.stringify({ kind: other })}`;
        if (!this.aiStatus.has(k)) this.aiStatus.set(k, { state: "found" });
      }
    } catch (e) {
      if (here()) this.aiStatus.set(key, { state: "error", message: String(e) });
    } finally {
      if (here()) {
        this.aiProgress = null;
        this.aiEpoch++;
      }
    }
  }

  /** The layers came back without some AI rasters (new library, cleared cache): detect them. */
  detectMissing(indices: number[]) {
    for (const i of indices) {
      const m = this.masks[i];
      const key = m && aiKey(m);
      const st = key ? this.aiStatus.get(key)?.state : undefined;
      // Don't retry failures automatically, or loop on a model that found nothing.
      if (m && st !== "working" && st !== "error" && st !== "empty") {
        if (st === "found") this.aiStatus.delete(key!); // cache was cleared under us
        this.confirmModel((m.shape as { model: string }).model).then((ok) => {
          if (ok) this.detect(m);
        });
      }
    }
  }

  deleteMask(i: number) {
    if (!this.edit || i >= this.masks.length) return;
    this.edit.masks = this.masks.filter((_, k) => k !== i);
    const n = this.edit.masks.length;
    this.maskIndex = n ? Math.min(i, n - 1) : null;
    return this.commit(null);
  }

  get cropping() {
    return this.cropEntry !== null;
  }

  startCrop() {
    if (!this.edit || this.cropping) return;
    this.compare = null;
    this.masking = false;
    this.cropAspect = null; // the panel opens on "Free"
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

  /** The photo asked for last (it may still be loading). Stepping counts from here. */
  opening = $state<number | null>(null);
  private openLoop: Promise<void> | null = null;

  /**
   * Open a library photo. Latest wins: while one photo loads, newer requests replace older
   * ones, so stepping quickly through photos only develops the one you stop on.
   */
  open(id: number): Promise<void> {
    if (id === this.opening && this.openLoop) return this.openLoop;
    this.opening = id;
    this.loading = "Developing…";
    this.error = null;
    this.openLoop ??= this.loadLatest().finally(() => (this.openLoop = null));
    return this.openLoop;
  }

  private async loadLatest() {
    try {
      while (this.opening !== null && this.opening !== this.image?.id) {
        const id = this.opening;
        try {
          const t0 = performance.now();
          const [info, checkout] = await Promise.all([api.openImage(id), api.history.open(id)]);
          if (this.opening !== id) continue;
          const pixels = await api.previewPixels(info.id);
          if (this.opening !== id) continue;
          console.info(`[tonality] opened ${info.name} ${info.width}×${info.height} in ${Math.round(performance.now() - t0)}ms`);
          this.pixels = pixels;
          this.compare = null;
          this.applying = null;
          this.cropEntry = null;
          this.cropAspect = null;
          this.masking = false;
          this.maskIndex = null;
          this.lassoing = null;
          this.aiStatus.clear();
          this.aiProgress = null;
          this.edit = checkout.state;
          this.history = checkout.view;
          this.image = info;
          // Refresh the grid's rendering of an edited photo (its history may have arrived from a
          // sidecar). An unedited one shows the plain thumbnail already: nothing to render.
          const head = checkout.view.commits.find((c) => c.id === checkout.view.head.commit);
          if (head?.parent != null) scheduleLibraryThumb(id, checkout.state);
        } catch (e) {
          if (this.opening !== id) continue;
          this.error = String(e);
          this.opening = this.image?.id ?? null;
        }
      }
    } finally {
      this.loading = null;
    }
  }

  /** Queued operations that will replace `edit` with their result (undo, checkout, …). */
  private restoring = 0;

  private run(op: (imageId: number) => Promise<Checkout>, applyState: boolean): Promise<void> {
    // The photo is the one on screen when asked, even if another opens before this runs.
    const id = this.history?.imageId;
    if (id === undefined) return Promise.resolve();
    if (applyState) this.restoring++;
    const next = this.queue.then(async () => {
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
      } finally {
        if (applyState) this.restoring--;
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
    // Behind an undo (or checkout) that hasn't landed, `edit` is about to be replaced: record
    // what's on screen once it has, so HEAD and the screen agree.
    const state = this.restoring ? null : $state.snapshot(this.edit);
    // Don't overwrite `edit` with the reply: the user may already be dragging again.
    return this.run((id) => {
      const s = state ?? (this.edit && this.history?.imageId === id ? $state.snapshot(this.edit) : null);
      return s ? api.history.commit(id, s, key) : api.history.open(id);
    }, false);
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
