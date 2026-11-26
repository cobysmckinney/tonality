// Version thumbnails and the library thumbnail of the current version are rendered with
// whatever photo the live renderer happens to show. These tests pin that a picture is only ever
// saved or cached for the photo it is of, and that a thumbnail that can't be made isn't asked
// for again without pause.
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type { EditState } from "$lib/bindings/EditState";
import type { PreviewRenderer } from "./renderer";

function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}

const state = (exposure = 0) =>
  ({ light: { exposure }, effects: { clarity: 0, dehaze: 0 }, masks: [] }) as unknown as EditState;

const historyState = vi.fn(async (_commit: number) => state());
let params: () => Promise<Float32Array> = () => Promise.resolve(new Float32Array(4));
const developParams = vi.fn((_id: number, _state: EditState) => params());
const invoke = vi.fn(async (..._args: unknown[]) => {});
const invalidateThumb = vi.fn();

vi.mock("@tauri-apps/api/core", () => ({ invoke }));
vi.mock("$lib/library/thumbs", () => ({ invalidateThumb }));
vi.mock("$lib/api", () => ({
  api: {
    developParams,
    developCurve: vi.fn(async () => new Uint16Array(4)),
    maskLayers: vi.fn(async () => null),
    presenceMaps: vi.fn(async () => ({})),
    history: { state: historyState },
  },
}));

/** A renderer showing photo `imageId`, whose stills say which photo they are of. */
function rendererOf(imageId: number) {
  const renderStill = vi.fn(() => ({
    toDataURL: () => `data:still-of-${imageId}`,
    toBlob: (done: (b: Blob) => void) => done(new Blob([new Uint8Array([imageId])])),
  }));
  return {
    renderer: { imageId, hasPresence: () => true, setPresence: vi.fn(), renderStill },
    renderStill,
  };
}
const attach = (v: Versions, r: ReturnType<typeof rendererOf> | null, id: number | null) =>
  v.attachRenderer((r?.renderer ?? null) as unknown as PreviewRenderer | null, id);

type Versions = typeof import("./versions");
let v: Versions;
beforeEach(async () => {
  vi.useFakeTimers();
  vi.resetModules();
  for (const f of [historyState, developParams, invoke, invalidateThumb]) f.mockClear();
  params = () => Promise.resolve(new Float32Array(4));
  v = await import("./versions");
});
afterEach(() => vi.useRealTimers());

/** Run everything due, timers included. */
const settle = () => vi.advanceTimersByTimeAsync(5_000);
/** The photos `library_save_edited` was called for. */
const saved = () =>
  invoke.mock.calls
    .filter((c) => c[0] === "library_save_edited")
    .map((c) => Number((c[2] as { headers: Record<string, string> }).headers["x-image-id"]));

describe("version thumbnails", () => {
  it("renders one once, with the renderer that shows its photo", async () => {
    const r = rendererOf(1);
    attach(v, r, 1);
    v.requestBranchThumb(1, 5, 100);
    v.requestBranchThumb(1, 5, 100);
    await settle();
    expect(v.branchThumbs.get(v.branchThumbKey(1, 5, 100))).toBe("data:still-of-1");
    expect(historyState).toHaveBeenCalledTimes(1);
  });

  it("renders a step again once later nudges have been merged into it", async () => {
    attach(v, rendererOf(1), 1);
    v.requestBranchThumb(1, 5, 100);
    await settle();
    // The same commit, changed in place: same id, later time.
    v.requestBranchThumb(1, 5, 900);
    await settle();
    expect(historyState).toHaveBeenCalledTimes(2);
    expect(v.branchThumbs.has(v.branchThumbKey(1, 5, 900))).toBe(true);
  });

  it("asks once for one that can't be rendered yet, and again when a renderer shows the photo", async () => {
    // No renderer (Edit still starting, or the GPU context was lost).
    v.requestBranchThumb(1, 5, 100);
    await settle();
    const key = v.branchThumbKey(1, 5, 100);
    // The placeholder stays: the panels that ask for thumbnails read this map, and removing
    // the key made them ask again at once, in a loop of backend calls.
    expect(v.branchThumbs.get(key)).toBe("");
    for (let i = 0; i < 5; i++) v.requestBranchThumb(1, 5, 100);
    await settle();
    expect(historyState).toHaveBeenCalledTimes(1);

    attach(v, rendererOf(1), 1);
    await settle();
    expect(historyState).toHaveBeenCalledTimes(2);
    expect(v.branchThumbs.get(key)).toBe("data:still-of-1");
  });

  it("doesn't ask again without pause when the backend refuses", async () => {
    attach(v, rendererOf(1), 1);
    historyState.mockRejectedValueOnce("image not open");
    v.requestBranchThumb(1, 5, 100);
    await settle();
    for (let i = 0; i < 5; i++) v.requestBranchThumb(1, 5, 100);
    await settle();
    expect(historyState).toHaveBeenCalledTimes(1);
    expect(v.branchThumbs.get(v.branchThumbKey(1, 5, 100))).toBe("");
    // Showing the photo anew is the one thing that tries again.
    attach(v, rendererOf(1), 1);
    await settle();
    expect(v.branchThumbs.get(v.branchThumbKey(1, 5, 100))).toBe("data:still-of-1");
  });

  it("never caches a still of another photo", async () => {
    // Asked for photo 1 while the renderer shows photo 2.
    attach(v, rendererOf(2), 2);
    v.requestBranchThumb(1, 5, 100);
    await settle();
    expect(v.branchThumbs.get(v.branchThumbKey(1, 5, 100))).toBe("");
  });
});

describe("the library thumbnail of the current version", () => {
  it("is saved a little after HEAD moves, for the photo on screen", async () => {
    const r = rendererOf(1);
    attach(v, r, 1);
    v.scheduleLibraryThumb(1, state(1));
    expect(saved()).toEqual([]);
    await settle();
    expect(saved()).toEqual([1]);
    expect(invalidateThumb).toHaveBeenCalledWith(1);
  });

  it("is rendered at once when the photo is left, while its pixels are still shown", async () => {
    const one = rendererOf(1);
    attach(v, one, 1);
    v.scheduleLibraryThumb(1, state(1));
    await Promise.resolve(); // its inputs are back; the delay hasn't passed
    await vi.advanceTimersByTimeAsync(0);
    // Another photo opens: the renderer is about to show it instead.
    attach(v, rendererOf(2), 2);
    expect(one.renderStill).toHaveBeenCalledTimes(1);
    await settle();
    expect(saved()).toEqual([1]);
  });

  it("is never rendered from the next photo's pixels when its inputs come back late", async () => {
    const late = deferred<Float32Array>();
    params = () => late.promise;
    attach(v, rendererOf(1), 1);
    v.scheduleLibraryThumb(1, state(1));
    // Photo 1 is left before its render inputs arrive: nothing can be rendered for it now.
    const two = rendererOf(2);
    attach(v, two, 2);
    late.resolve(new Float32Array(4));
    await settle();
    expect(two.renderStill).not.toHaveBeenCalled();
    expect(saved()).toEqual([]);

    // Back on photo 1, the save that couldn't run is made after all.
    const one = rendererOf(1);
    attach(v, one, 1);
    await settle();
    expect(one.renderStill).toHaveBeenCalledTimes(1);
    expect(saved()).toEqual([1]);
  });

  it("is not saved when Edit closes before the inputs arrive, nor from nothing", async () => {
    const late = deferred<Float32Array>();
    params = () => late.promise;
    attach(v, rendererOf(1), 1);
    v.scheduleLibraryThumb(1, state(1));
    attach(v, null, null);
    late.resolve(new Float32Array(4));
    await settle();
    expect(saved()).toEqual([]);
  });

  it("is dropped when the photo's history is reset meanwhile", async () => {
    attach(v, rendererOf(1), 1);
    v.scheduleLibraryThumb(1, state(1));
    await v.dropLibraryThumbs([1]);
    await settle();
    expect(saved()).toEqual([]);
  });
});
