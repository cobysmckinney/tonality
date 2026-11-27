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
import { invalidateThumb } from "$lib/library/thumbs";
import { toasts } from "$lib/toasts/toasts.svelte";
import { dropLibraryThumbs, scheduleLibraryThumb } from "./versions";
import { afterHistory, afterOpen, afterRemoved, type Compare } from "./compare";
import {
  MAX_MASKS,
  aiKey,
  masksOf,
  newAiMask,
  newMask,
  type AiPreset,
  type BrushTool,
  type MaskKind,
} from "./masks";

/** Where an AI mask's detection stands (keyed by `aiKey`). `declined`: its model isn't
 *  installed and the download was turned down, for this photo. */
export type AiStatus =
  | { state: "working" }
  | { state: "found" }
  | { state: "empty" }
  | { state: "error" }
  | { state: "declined" };

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
  /** A photo's file name, for messages about one that isn't open (set by the Library). */
  nameOf: (id: number) => string | undefined = () => undefined;
  /** Side-by-side compare with another branch (its tip), or with another photo. */
  compare = $state<Compare | null>(null);
  /** Version the Merge dialog is merging from. */
  applying = $state<string | null>(null);
  /** Shown on the canvas instead of `edit` (hovering a version); never committed. */
  preview = $state.raw<{ label: string; state: EditState } | null>(null);
  /** Crop mode: the crop as it was on entering (for Cancel). Changes commit on Done. */
  cropEntry = $state<Crop | null>(null);
  /** Locked crop aspect (pixel w/h), or null for free. */
  cropAspect = $state<number | null>(null);

  /** Mask mode: the mask list replaces the adjustments, and the selected mask has handles. */
  masking = $state(false);
  maskIndex = $state<number | null>(null);
  /** Tint the selected mask's area red (O). */
  maskOverlay = $state(true);
  /** The mask the pointer is over in the mask list: its area shows while hovering. */
  maskHover = $state<number | null>(null);
  /** Each mask on its own in black and white (null: hidden, so not drawn), set by Canvas. */
  maskThumbs = $state.raw<(ImageData | null)[]>([]);
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
    this.setCompare(null);
    this.masking = true;
    if (this.selected === null && this.masks.length) this.maskIndex = this.masks.length - 1;
  }

  stopMasks() {
    this.masking = false;
    this.lassoing = null;
    this.maskHover = null;
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

  /** The download prompt that is open for each model, so two masks waiting on one model
   *  (Sky and People, Subject and Background) share one prompt, not one each. */
  #asking = new Map<string, Promise<boolean>>();

  /** Ask before a model's first download. Resolves false if declined. */
  private confirmModel(model: string): Promise<boolean> {
    const open = this.#asking.get(model);
    if (open) return open;
    const asked = this.askForModel(model).finally(() => this.#asking.delete(model));
    this.#asking.set(model, asked);
    return asked;
  }

  private async askForModel(model: string): Promise<boolean> {
    if (this.approved.has(model)) return true;
    let info;
    try {
      info = (await api.ai.models()).find((m) => m.id === model);
    } catch (e) {
      toasts.error("Couldn’t look up the AI models", e);
      return false;
    }
    if (!info || info.installed) {
      this.approved.add(model);
      return true;
    }
    const mb = Math.round(info.size / 1e6);
    const ok = await ask(
      `${info.label} masks need a one-time ${mb} MB download. The model runs on this computer: your photos never leave it.\n\nThe model’s licence: ${info.license}.`,
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
    // Nothing is committed in the middle of a crop.
    if (this.cropping) return;
    const model = await api.ai.modelFor(target);
    if (!(await this.confirmModel(model)) || !this.edit || this.cropping) return;
    const m = newAiMask(preset, target, model, this.masks);
    this.edit.masks = [...this.masks, m];
    this.maskIndex = this.edit.masks.length - 1;
    this.commit(null);
    await this.detect(m);
  }

  startLasso(replace: number | null = null) {
    if (!this.edit || this.cropping) return;
    if (replace === null && this.masks.length >= MAX_MASKS) return;
    this.lassoing = { replace };
  }

  cancelLasso() {
    this.lassoing = null;
  }

  /** A loop was drawn (source uv): select the object in it. */
  async finishLasso(lasso: [number, number][]) {
    const job = this.lassoing;
    if (!job || job.busy) return;
    if (!this.edit || this.cropping || lasso.length < 3) {
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

  /**
   * Run the model for an AI mask, unless it's already running. `lens`: the lens settings of
   * the state the mask is in, when that isn't the current edit (a previewed version).
   */
  async detect(m: Mask, lens = this.edit?.lens ?? null) {
    const key = aiKey(m);
    const id = this.image?.id;
    if (
      !key ||
      id === undefined ||
      m.shape.kind !== "ai" ||
      this.aiStatus.get(key)?.state === "working"
    )
      return;
    const { target, model } = $state.snapshot(m.shape);
    this.aiStatus.set(key, { state: "working" });
    // Statuses are per photo (cleared on open): a result for one we've left is dropped.
    const here = () => this.image?.id === id;
    try {
      const found = await api.ai.detect(id, target, model, lens);
      if (!here()) return;
      this.aiStatus.set(key, { state: found ? "found" : "empty" });
      // A scene pass also caches its sibling (sky ↔ people).
      if (target.kind === "sky" || target.kind === "people") {
        const other = target.kind === "sky" ? "people" : "sky";
        const k = `${model}:${JSON.stringify({ kind: other })}`;
        if (!this.aiStatus.has(k)) this.aiStatus.set(k, { state: "found" });
      }
    } catch (e) {
      // Kept as a state too: a failed mask isn't retried by itself, and offers Try Again.
      if (here()) {
        this.aiStatus.set(key, { state: "error" });
        toasts.error(`Couldn’t make the “${m.name}” mask`, e);
      }
    } finally {
      if (here()) {
        this.aiProgress = null;
        this.aiEpoch++;
      }
    }
  }

  /**
   * The layers came back without some AI rasters (new library, cleared cache): detect them.
   * `masks` are the masks themselves, from the state the layers were fetched for, with its
   * `lens`. That state isn't always the current edit (a version being previewed, the compare
   * pane), so they can't be looked up by position in `this.masks`.
   */
  detectMissing(masks: Mask[], lens: EditState["lens"] | null = this.edit?.lens ?? null) {
    for (const m of masks) {
      const key = aiKey(m);
      if (!key || m.shape.kind !== "ai") continue;
      const st = this.aiStatus.get(key)?.state;
      // Don't retry failures automatically, loop on a model that found nothing, or ask again
      // for a download that was just turned down.
      if (st === "working" || st === "error" || st === "empty" || st === "declined") continue;
      if (st === "found") this.aiStatus.delete(key); // cache was cleared under us
      void this.downloadAndDetect(m, lens);
    }
  }

  /**
   * Detect an AI mask whose model may not be installed: ask about the download first (again,
   * if it was declined before: this is also the mask panel's Download button).
   */
  async downloadAndDetect(m: Mask, lens = this.edit?.lens ?? null) {
    const key = aiKey(m);
    if (!key || m.shape.kind !== "ai") return;
    const id = this.image?.id;
    const ok = await this.confirmModel(m.shape.model);
    // Statuses are per photo: an answer for one we've left says nothing about this one.
    if (this.image?.id !== id) return;
    if (ok) {
      if (this.aiStatus.get(key)?.state === "declined") this.aiStatus.delete(key);
      await this.detect(m, lens);
    } else {
      this.aiStatus.set(key, { state: "declined" });
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
    this.setCompare(null);
    // All of mask mode, a Select Object loop waiting to be drawn included: left armed, it sat
    // over the crop box, took every press, and the mask it added committed the unfinished crop.
    this.stopMasks();
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

  async defaultEdit(): Promise<EditState> {
    this.defaults ??= await api.defaultEditState();
    return structuredClone(this.defaults);
  }

  /** Told the state a photo's HEAD shows, each time it's opened or its history answers. */
  onHead: ((imageId: number, state: EditState) => void) | null = null;

  /** The photo asked for last (it may still be loading). Stepping counts from here. */
  opening = $state<number | null>(null);
  private openLoop: Promise<void> | null = null;

  /**
   * Open a library photo. Latest wins: while one photo loads, newer requests replace older
   * ones, so stepping quickly through photos only develops the one you stop on.
   */
  open(id: number): Promise<void> {
    if (id === this.opening && this.openLoop) return this.openLoop;
    if (id !== this.image?.id) this.keepCrop();
    this.opening = id;
    this.loading = "Developing…";
    this.openLoop ??= this.loadLatest().finally(() => (this.openLoop = null));
    return this.openLoop;
  }

  private async loadLatest() {
    // Opens given up on, because another photo was asked for meanwhile, since the last one
    // that was seen through.
    let abandoned = 0;
    try {
      while (this.opening !== null && this.opening !== this.image?.id) {
        const id = this.opening;
        try {
          const t0 = performance.now();
          const [info, checkout] = await Promise.all([api.openImage(id), api.history.open(id)]);
          if (this.opening !== id) {
            abandoned++;
            continue;
          }
          const pixels = await api.previewPixels(info.id);
          if (this.opening !== id) {
            abandoned++;
            continue;
          }
          console.info(
            `[tonality] opened ${info.name} ${info.width}×${info.height} in ${Math.round(performance.now() - t0)}ms`,
          );
          abandoned = 0; // this photo is now the one the backend used last
          this.pixels = pixels;
          this.setCompare(afterOpen(this.compare, id));
          this.applying = null;
          this.preview = null;
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
          this.onHead?.(id, checkout.state);
          // Refresh the grid's rendering of an edited photo (its history may have arrived from a
          // sidecar). An unedited one shows the plain thumbnail already: nothing to render.
          const head = checkout.view.commits.find((c) => c.id === checkout.view.head.commit);
          if (head?.parent != null) scheduleLibraryThumb(id, checkout.state);
        } catch (e) {
          if (this.opening !== id) {
            abandoned++;
            continue;
          }
          toasts.error(`Couldn’t open ${this.nameOf(id) ?? "this photo"}`, e);
          this.opening = this.image?.id ?? null;
        }
      }
      // Ended on the photo already on screen, after stepping away and straight back: the
      // backend decoded each photo that was given up on, and it only keeps a few. Open this
      // one again, so that it is the one used last there (it's still decoded, almost always)
      // and can't be the next to go, which left it unable to redraw: "image not open".
      const shown = this.image?.id;
      if (abandoned && shown !== undefined && this.opening === shown) {
        await api.openImage(shown).catch((e) => toasts.error("Couldn’t reopen this photo", e));
      }
    } finally {
      this.loading = null;
    }
  }

  /** Photos were removed from the library: stop showing (or loading) any of them. */
  forget(ids: number[]) {
    this.compare = afterRemoved(this.compare, ids); // (its pixels are dropped already)
    if (this.opening !== null && ids.includes(this.opening)) {
      this.opening = this.image?.id ?? null;
    }
    if (!this.image || !ids.includes(this.image.id)) return;
    if (this.opening === this.image.id) this.opening = null;
    this.image = null;
    this.pixels = null;
    this.edit = null;
    this.history = null;
    this.setCompare(null);
    this.applying = null;
    this.preview = null;
    this.cropEntry = null;
    this.cropAspect = null;
    this.masking = false;
    this.maskIndex = null;
    this.lassoing = null;
  }

  /** Queued operations that will replace `edit` with their result (undo, checkout, …). */
  private restoring = 0;

  private run(
    op: (imageId: number) => Promise<Checkout>,
    applyState: boolean,
    // The photo is the one on screen when asked, even if another opens before this runs.
    id: number | undefined = this.history?.imageId,
  ): Promise<void> {
    if (id === undefined) return Promise.resolve();
    if (applyState) this.restoring++;
    const next = this.queue.then(async () => {
      try {
        const r = await op(id);
        if (this.history?.imageId !== id) return; // another image was opened meanwhile
        // Another step, or the same one changed: a control nudged again soon after is merged
        // into its step in place, which keeps the id and changes the look (and the time).
        const moved =
          this.history.head.commit !== r.view.head.commit ||
          headTime(this.history) !== headTime(r.view);
        this.history = r.view;
        this.onHead?.(id, r.state);
        if (applyState) {
          this.edit = r.state;
          // Mid-crop, Cancel goes back to the crop of the state now shown, not the one replaced.
          if (this.cropEntry) this.cropEntry = $state.snapshot(r.state.crop);
        }
        if (moved) scheduleLibraryThumb(id, r.state);
        if (this.compare) this.compare = afterHistory(this.compare, r.view);
      } catch (e) {
        toasts.error("Couldn’t update the history", e);
        throw e;
      } finally {
        if (applyState) this.restoring--;
      }
    });
    this.queue = next.catch(() => {});
    return next;
  }

  /** Record the current edit as a commit. `key` names the control, so rapid nudges coalesce. */
  commit(key: string | null) {
    if (!this.edit) return Promise.resolve();
    // Behind an undo (or checkout) that hasn't landed, `edit` is about to be replaced: record
    // what's on screen once it has, so HEAD and the screen agree.
    const state = this.restoring ? null : $state.snapshot(this.edit);
    // Don't overwrite `edit` with the reply: the user may already be dragging again.
    return this.run((id) => {
      const s =
        state ?? (this.edit && this.history?.imageId === id ? $state.snapshot(this.edit) : null);
      return s ? api.history.commit(id, s, key) : api.history.open(id);
    }, false);
  }

  undo = () => this.run((id) => api.history.undo(id), true);
  redo = () => this.run((id) => api.history.redo(id), true);
  checkout = (commit: number) => this.run((id) => api.history.checkout(id, commit), true);
  switchBranch = (name: string) => {
    this.keepCrop();
    return this.run((id) => api.history.switchBranch(id, name), true);
  };
  branch = (at: number, name: string | null = null) => {
    this.keepCrop();
    return this.run((id) => api.history.branch(id, at, name), true);
  };

  /**
   * A crop in progress is about to be left behind (another photo, another version): finish it
   * first, as Done would. It is only recorded on Done, so walking away used to drop it without
   * a word and with nothing in history to get it back from. Queued ahead of what follows, for
   * the photo and state on screen now.
   */
  private keepCrop() {
    if (this.cropping) void this.finishCrop();
  }
  renameBranch = (old: string, name: string) => {
    if (this.compare?.kind === "version" && this.compare.branch === old)
      this.compare.branch = name.trim();
    return this.run((id) => api.history.rename(id, old, name), true);
  };
  deleteBranch = (name: string) => this.run((id) => api.history.deleteBranch(id, name), true);
  applyFrom = (source: string, panels: Panel[]) =>
    this.run((id) => api.history.applyFrom(id, source, panels), true);

  /**
   * The open photo's history was reset (here or from the Library): load the fresh one, and
   * drop anything that pointed into the old history.
   */
  async reloadHistory(id: number | undefined = this.history?.imageId): Promise<void> {
    // Another photo was opened meanwhile: its state isn't ours to clear.
    if (id === undefined || this.history?.imageId !== id) return;
    if (this.compare?.kind === "version") this.compare = null; // (a reference photo stays)
    this.applying = null;
    this.cropEntry = null;
    this.masking = false;
    this.maskIndex = null;
    this.lassoing = null;
    await this.run((i) => api.history.open(i), true, id);
    // The fresh HEAD is as-shot: the plain thumbnail is right, so don't render an edited one.
    await dropLibraryThumbs([id]);
    invalidateThumb(id);
  }

  /** Delete every version and step of the open photo and start again from as-shot. */
  async resetHistory(): Promise<void> {
    const id = this.history?.imageId;
    if (id === undefined) return;
    // After any edit still being recorded, so it can't land on the fresh history.
    await this.queue;
    await dropLibraryThumbs([id]);
    try {
      await api.history.reset([id]);
    } catch (e) {
      toasts.error("Couldn’t reset the history", e);
      return;
    }
    if (this.history?.imageId === id) await this.reloadHistory(id);
    else {
      // Left meanwhile: nothing to reload, but its library thumbnail is as-shot again.
      await dropLibraryThumbs([id]);
      invalidateThumb(id);
    }
  }

  /** Show `branch` next to the current version. */
  async startCompare(branch: string) {
    const b = this.history?.branches.find((x) => x.name === branch);
    if (!b || branch === this.history?.head.branch) return;
    try {
      const state = await api.history.state(b.tip);
      // As when comparing with another photo: the brush and the mask handles map the pointer
      // across the whole canvas, not the half the edited photo is then drawn in.
      this.stopMasks();
      this.setCompare({ kind: "version", branch, state });
    } catch (e) {
      toasts.error(`Couldn’t show “${branch}”`, e);
    }
  }

  private referenceSeq = 0;

  /**
   * Show photo `id` next to the one being edited, read-only, at its current version (a
   * reference, to match a look across a set). It stays while switching photos, until closed or
   * opened itself. Its history is only read: nothing is created or changed for it.
   */
  async compareWith(id: number) {
    const open = this.image?.id;
    if (open === undefined || id === open || id === this.opening || this.cropping) return;
    if (this.compare?.kind === "photo" && this.compare.image.id === id) return;
    const seq = ++this.referenceSeq;
    try {
      const [image, head] = await Promise.all([api.openReference(id), api.history.head(id)]);
      const [pixels, state] = await Promise.all([
        api.previewPixels(id),
        head ? api.history.state(head.commit) : this.defaultEdit(),
      ]);
      if (seq !== this.referenceSeq) return; // a newer one was asked for, or it was closed
      // Opened meanwhile (or nothing is open any more): nothing to compare it with.
      if (id === this.image?.id || id === this.opening || !this.image || this.cropping) {
        api.closeReference(id).catch(() => {});
        return;
      }
      // Mask handles belong over the photo being edited, which moves to the right.
      if (this.masking) this.stopMasks();
      this.setCompare({ kind: "photo", image, pixels, version: head?.branch ?? "main", state });
    } catch (e) {
      if (seq === this.referenceSeq) {
        toasts.error(`Couldn’t compare with ${this.nameOf(id) ?? "that photo"}`, e);
      }
    }
  }

  stopCompare() {
    this.referenceSeq++; // (one still loading doesn't open)
    this.setCompare(null);
  }

  /** Replace the compare. A reference photo that goes is no longer held for it. */
  private setCompare(next: Compare | null) {
    const prev = this.compare;
    if (prev?.kind === "photo" && next !== prev) api.closeReference(prev.image.id).catch(() => {});
    this.compare = next;
  }
}

/** When the commit HEAD is on was last changed. */
function headTime(v: HistoryView) {
  return v.commits.find((c) => c.id === v.head.commit)?.time;
}

export const session = new EditSession();
