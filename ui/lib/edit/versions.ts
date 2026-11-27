// Rendered stills of an image's versions, made with the live preview renderer:
// - small thumbnails of each branch tip (branch switcher), cached per commit;
// - the current version's library thumbnail, saved to the backend whenever HEAD moves, so the
//   Library grid shows photos as edited.
import { invoke } from "@tauri-apps/api/core";
import { SvelteMap } from "svelte/reactivity";
import { api } from "$lib/api";
import type { EditState } from "$lib/bindings/EditState";
import { invalidateThumb } from "$lib/library/thumbs";
import type { PreviewRenderer } from "./renderer";
import { brushKey, usesPresenceMaps, withoutStrokes } from "./masks";

const BRANCH_EDGE = 200;
const BRANCH_THUMBS = 64;
const LIBRARY_EDGE = 640;
const SAVE_DELAY_MS = 700;

let renderer: PreviewRenderer | null = null;
let rendererImage: number | null = null;

/** A version thumbnail's key: `${imageId}:${commit}:${time}` → data URL ("" while it's being
 *  made, or couldn't be). `time` is the commit's: repeated nudges of one control are merged
 *  into the same commit, which keeps its id while its look changes. */
export const branchThumbs = new SvelteMap<string, string>();
export const branchThumbKey = (imageId: number, commit: number, time: number) =>
  `${imageId}:${commit}:${time}`;
/** Couldn't be rendered when asked for (no renderer showing their photo yet, or a backend call
 *  failed), by key. Tried again when a renderer shows their photo, and only then. */
const waiting = new Map<string, { imageId: number; commit: number; time: number }>();

let chain: Promise<unknown> = Promise.resolve();
function serial<T>(f: () => Promise<T>): Promise<T> {
  const next = chain.then(f, f);
  chain = next.catch(() => {});
  return next;
}

/** Everything needed to render `state`: params, curve and brush rasters. */
function inputs(imageId: number, state: EditState) {
  return Promise.all([
    api.developParams(imageId, withoutStrokes(state)),
    api.developCurve(state),
    brushKey(state) ? api.maskLayers(imageId, state) : null,
  ]);
}

/** Whether the attached renderer has `imageId`'s pixels uploaded (and can render it). */
function shows(imageId: number): boolean {
  return !!renderer && rendererImage === imageId && renderer.imageId === imageId;
}

async function still(imageId: number, state: EditState, edge: number) {
  const [params, curve, brush] = await inputs(imageId, state);
  if (!renderer || !shows(imageId)) return null;
  if (usesPresenceMaps(state) && !renderer.hasPresence(imageId)) {
    const maps = await api.presenceMaps(imageId);
    if (!renderer || !shows(imageId)) return null;
    if (!renderer.hasPresence(imageId)) renderer.setPresence(imageId, maps);
  }
  return renderer.renderStill(edge, params, curve, brush);
}

/** Called by Canvas once its renderer shows `imageId` (null when it goes away). */
export function attachRenderer(r: PreviewRenderer | null, imageId: number | null) {
  // Leaving this photo (switching, or Edit closing): save its thumbnail while we still can.
  if (pending && pending.imageId !== imageId) leave();
  renderer = r;
  rendererImage = imageId;
  if (r) {
    const again = [...waiting.entries()].filter(([, t]) => t.imageId === imageId);
    waiting.clear();
    for (const [key, t] of again) {
      branchThumbs.delete(key);
      requestBranchThumb(t.imageId, t.commit, t.time);
    }
  }
  // Back on a photo whose save couldn't run when it was left: try it now.
  const old = imageId === null ? undefined : parked.get(imageId);
  if (r && old && !pending) {
    parked.delete(old.imageId);
    pending = old;
  }
  if (r && pending?.imageId === imageId) {
    clearTimeout(timer);
    timer = setTimeout(flushSave, SAVE_DELAY_MS);
  }
}

/** Dev: PNG data URL of the next frame the Edit canvas draws. */
export function snapshotNextFrame(): Promise<string> {
  return new Promise((resolve, reject) => {
    const r = renderer;
    if (!r) return reject(new Error("no renderer"));
    const prev = r.onDrawn;
    r.onDrawn = (c) => {
      r.onDrawn = prev;
      resolve(c.toDataURL("image/png"));
    };
    r.requestDraw();
  });
}

/** Thumbnail of the version at `commit`, as it was at `time` (rendered once, then cached). */
export function requestBranchThumb(imageId: number, commit: number, time: number) {
  const key = branchThumbKey(imageId, commit, time);
  if (branchThumbs.has(key)) return;
  // Keep one photo's thumbnails, and not too many of those (one per commit asked for).
  for (const k of branchThumbs.keys()) if (!k.startsWith(`${imageId}:`)) branchThumbs.delete(k);
  while (branchThumbs.size >= BRANCH_THUMBS) branchThumbs.delete(branchThumbs.keys().next().value!);
  branchThumbs.set(key, ""); // pending
  serial(async () => {
    const state = await api.history.state(commit);
    const c = await still(imageId, state, BRANCH_EDGE);
    if (c) branchThumbs.set(key, c.toDataURL("image/jpeg", 0.85));
    // No renderer for this photo yet (Edit still starting, or the GPU context is lost): try
    // again once there is one.
    else waiting.set(key, { imageId, commit, time });
  }).catch(() => {
    waiting.set(key, { imageId, commit, time });
  });
  // Either way the placeholder stays. The panels that ask for thumbnails read this map, so
  // removing the key would run them again at once, and they'd ask again, without pause, for as
  // long as whatever is wrong lasts.
}

// ---- library thumbnail of the current version ----

type Inputs = Awaited<ReturnType<typeof inputs>>;
type SaveJob = { imageId: number; inputs: Inputs | null; failed: boolean };
let pending: SaveJob | null = null;
/** Saves for photos left before they could render (inputs not back yet, or no GPU context);
 *  picked up again when the photo is shown. */
const parked = new Map<number, SaveJob>();
const MAX_PARKED = 16;
let timer: ReturnType<typeof setTimeout> | undefined;

/**
 * HEAD changed: refresh the library thumbnail soon. Its inputs are fetched right away, so that
 * when the renderer is about to move on (another photo, Edit closing) it can render at once.
 */
export function scheduleLibraryThumb(imageId: number, state: EditState) {
  // The previous photo's renderer still shows it (the new one isn't uploaded yet): save it now.
  if (pending && pending.imageId !== imageId) leave();
  parked.delete(imageId); // superseded
  const job: SaveJob = { imageId, inputs: null, failed: false };
  pending = job;
  inputs(imageId, state).then(
    (v) => {
      job.inputs = v;
      if (pending === job && !timer) flushSave(); // came back after the delay
    },
    (e) => {
      job.failed = true;
      console.warn("[versions] library thumbnail inputs:", e);
    },
  );
  clearTimeout(timer);
  timer = setTimeout(() => {
    timer = undefined;
    flushSave();
  }, SAVE_DELAY_MS);
}

/**
 * Forget any library thumbnail about to be saved for `ids` (their history is being reset) and
 * wait for a save already under way, so none lands after the reset.
 */
export async function dropLibraryThumbs(ids: number[]): Promise<void> {
  if (pending && ids.includes(pending.imageId)) {
    clearTimeout(timer);
    timer = undefined;
    pending = null;
  }
  for (const id of ids) parked.delete(id);
  await serial(async () => {}).catch(() => {});
}

/** The pending job's photo is going off screen: render it now, or park it for later. */
function leave() {
  flushSave();
  const job = pending;
  if (!job) return;
  pending = null;
  parked.delete(job.imageId);
  parked.set(job.imageId, job);
  while (parked.size > MAX_PARKED) parked.delete(parked.keys().next().value!);
}

/** Render the pending job now, synchronously, if its photo is on screen and inputs are in. */
function flushSave() {
  clearTimeout(timer);
  timer = undefined;
  const job = pending;
  if (!job) return;
  if (job.failed) return void (pending = null);
  // Not ready, or not on screen: kept pending (retried when the inputs land or on attach).
  if (!job.inputs || !renderer || !shows(job.imageId)) return;
  const c = renderer.renderStill(LIBRARY_EDGE, ...job.inputs);
  if (!c) return; // no pixels or GPU context: kept pending
  pending = null;
  serial(async () => {
    const blob = await new Promise<Blob | null>((res) => c.toBlob(res, "image/jpeg", 0.88));
    if (!blob) return;
    await invoke("library_save_edited", new Uint8Array(await blob.arrayBuffer()), {
      headers: { "x-image-id": String(job.imageId) },
    });
    invalidateThumb(job.imageId);
  }).catch((e) => console.warn("[versions] library thumbnail:", e));
}
