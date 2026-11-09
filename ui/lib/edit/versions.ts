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
import { brushKey } from "./masks";

const BRANCH_EDGE = 200;
const LIBRARY_EDGE = 640;
const SAVE_DELAY_MS = 700;

let renderer: PreviewRenderer | null = null;
let rendererImage: number | null = null;

/** `${imageId}:${commit}` → data URL. */
export const branchThumbs = new SvelteMap<string, string>();

let chain: Promise<unknown> = Promise.resolve();
function serial<T>(f: () => Promise<T>): Promise<T> {
  const next = chain.then(f, f);
  chain = next.catch(() => {});
  return next;
}

/** Everything needed to render `state`: params, curve and brush rasters. */
function inputs(imageId: number, state: EditState) {
  return Promise.all([
    api.developParams(imageId, state),
    api.developCurve(state),
    brushKey(state) ? api.maskLayers(imageId, state) : null,
  ]);
}

async function still(imageId: number, state: EditState, edge: number) {
  const [params, curve, brush] = await inputs(imageId, state);
  if (!renderer || rendererImage !== imageId) return null;
  return renderer.renderStill(edge, params, curve, brush);
}

/** Called by Canvas once its renderer shows `imageId` (null when it goes away). */
export function attachRenderer(r: PreviewRenderer | null, imageId: number | null) {
  if (r === null) flushSave();
  renderer = r;
  rendererImage = imageId;
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

/** Thumbnail of the version at `commit` (rendered once, then cached). */
export function requestBranchThumb(imageId: number, commit: number) {
  const key = `${imageId}:${commit}`;
  if (branchThumbs.has(key)) return;
  branchThumbs.set(key, ""); // pending
  serial(async () => {
    const state = await api.history.state(commit);
    const c = await still(imageId, state, BRANCH_EDGE);
    if (c) branchThumbs.set(key, c.toDataURL("image/jpeg", 0.85));
    else branchThumbs.delete(key);
  }).catch(() => branchThumbs.delete(key));
}

// ---- library thumbnail of the current version ----

let pending: { imageId: number; state: EditState } | null = null;
let timer: ReturnType<typeof setTimeout> | undefined;

/** HEAD changed: refresh the library thumbnail soon. */
export function scheduleLibraryThumb(imageId: number, state: EditState) {
  pending = { imageId, state };
  clearTimeout(timer);
  timer = setTimeout(flushSave, SAVE_DELAY_MS);
}

function flushSave() {
  clearTimeout(timer);
  const job = pending;
  if (!job || !renderer || rendererImage !== job.imageId) return; // retried on attach
  pending = null;
  // Render synchronously now (the renderer may be about to go away), upload afterwards.
  const r = renderer;
  const canvasP = inputs(job.imageId, job.state).then(([params, curve, brush]) =>
    rendererImage === job.imageId && renderer === r ? r.renderStill(LIBRARY_EDGE, params, curve, brush) : null,
  );
  serial(async () => {
    const c = await canvasP;
    if (!c) return;
    const blob = await new Promise<Blob | null>((res) => c.toBlob(res, "image/jpeg", 0.88));
    if (!blob) return;
    await invoke("library_save_edited", new Uint8Array(await blob.arrayBuffer()), {
      headers: { "x-image-id": String(job.imageId) },
    });
    invalidateThumb(job.imageId);
  }).catch((e) => console.warn("[versions] library thumbnail:", e));
}
