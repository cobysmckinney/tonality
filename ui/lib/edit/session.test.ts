// The edit session decides what is shown and saved for which photo, over backend calls that
// answer in their own time. These tests hold those answers back and release them in the orders
// that have gone wrong before: a slower open finishing last, a reply for a photo that was left.
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { Checkout } from "$lib/bindings/Checkout";
import type { EditState } from "$lib/bindings/EditState";
import type { ImageInfo } from "$lib/bindings/ImageInfo";

/** A promise the test settles by hand. */
function deferred<T>() {
  let resolve!: (v: T) => void;
  let reject!: (e: unknown) => void;
  const promise = new Promise<T>((res, rej) => {
    resolve = res;
    reject = rej;
  });
  return { promise, resolve, reject };
}
type Deferred<T> = ReturnType<typeof deferred<T>>;

/** Calls to one backend command, each waiting for the test to answer it. */
function pendingCalls<A extends unknown[], T>() {
  const calls: { args: A; answer: Deferred<T> }[] = [];
  const fn = vi.fn((...args: A) => {
    const answer = deferred<T>();
    calls.push({ args, answer });
    return answer.promise;
  });
  return { fn, calls };
}

const openImage = pendingCalls<[number], ImageInfo>();
const historyOpen = pendingCalls<[number], Checkout>();
const commit = pendingCalls<[number, EditState, string | null], Checkout>();
const undo = pendingCalls<[number], Checkout>();
const previewPixels = vi.fn(async (id: number) => new Uint16Array([id]));
const scheduleLibraryThumb = vi.fn();
const error = vi.fn();

vi.mock("@tauri-apps/api/event", () => ({ listen: () => Promise.resolve(() => {}) }));
vi.mock("@tauri-apps/plugin-dialog", () => ({ ask: vi.fn(async () => true) }));
vi.mock("$lib/library/thumbs", () => ({ invalidateThumb: vi.fn() }));
vi.mock("$lib/toasts/toasts.svelte", () => ({ toasts: { error, notice: vi.fn() } }));
vi.mock("./versions", () => ({
  scheduleLibraryThumb,
  dropLibraryThumbs: vi.fn(async () => {}),
}));
vi.mock("$lib/api", () => ({
  api: {
    openImage: openImage.fn,
    previewPixels,
    history: { open: historyOpen.fn, commit: commit.fn, undo: undo.fn },
  },
}));

const info = (id: number) => ({ id, name: `IMG_${id}.ARW` }) as unknown as ImageInfo;
const edit = (exposure: number) =>
  ({
    light: { exposure },
    crop: { rect: [0, 0, 1, 1], angle: 0, turns: 0, flip: false },
  }) as unknown as EditState;
/** A photo's history with HEAD on commit `head`, last changed at `time`, showing `exposure`. */
const checkout = (imageId: number, head: number, exposure = 0, time = 100): Checkout => ({
  state: edit(exposure),
  view: {
    imageId,
    commits: [
      { id: 1, parent: null, summary: "Original", time: 0 },
      ...(head > 1 ? [{ id: head, parent: 1, summary: "Exposure", time }] : []),
    ],
    branches: [{ name: "main", tip: head }],
    head: { branch: "main", commit: head },
    canUndo: head > 1,
    canRedo: false,
  },
});
/** Let everything that is ready to run, run. */
const settle = async () => {
  for (let i = 0; i < 20; i++) await Promise.resolve();
};

async function fresh() {
  vi.resetModules();
  for (const c of [openImage, historyOpen, commit, undo]) {
    c.calls.length = 0;
    c.fn.mockClear();
  }
  previewPixels.mockClear();
  scheduleLibraryThumb.mockClear();
  error.mockClear();
  return (await import("./session.svelte")).session;
}
type Session = Awaited<ReturnType<typeof fresh>>;

/** Answer the open of photo `id` that is waiting (the `nth` one asked for, from 0). */
function answerOpen(id: number, nth = 0, c: Checkout = checkout(id, 1)) {
  openImage.calls.filter((x) => x.args[0] === id)[nth].answer.resolve(info(id));
  historyOpen.calls.filter((x) => x.args[0] === id)[nth].answer.resolve(c);
}
async function opened(session: Session, id: number, c: Checkout = checkout(id, 1)) {
  const done = session.open(id);
  answerOpen(id, 0, c);
  await done;
}

let session: Session;
beforeEach(async () => {
  session = await fresh();
});

describe("opening a photo", () => {
  it("shows the photo and its history", async () => {
    await opened(session, 1, checkout(1, 2, 0.5));
    expect(session.image?.id).toBe(1);
    expect(session.edit?.light.exposure).toBe(0.5);
    expect(session.history?.head.commit).toBe(2);
    expect(session.loading).toBeNull();
  });

  it("ends on the photo asked for last, whichever answers first", async () => {
    const first = session.open(1);
    session.open(2);
    // The first photo's answer arrives after the second was asked for: it is not shown...
    answerOpen(1);
    await settle();
    expect(session.image).toBeNull();
    expect(previewPixels).not.toHaveBeenCalled();
    // ...and the second is then loaded.
    answerOpen(2);
    await first;
    expect(session.image?.id).toBe(2);
    expect(previewPixels).toHaveBeenCalledExactlyOnceWith(2);
  });

  it("stepping away and straight back opens the shown photo again", async () => {
    // (The backend keeps only a few decoded photos, and decodes each one asked for: the photo
    // on screen has to be the one it used last, or it can be dropped while still shown.)
    await opened(session, 10);
    const away = session.open(9);
    session.open(10); // back before 9 has loaded
    answerOpen(9);
    await settle();
    expect(session.image?.id).toBe(10);
    const again = openImage.calls.filter((c) => c.args[0] === 10);
    expect(again).toHaveLength(2);
    again[1].answer.resolve(info(10));
    await away;
    expect(session.image?.id).toBe(10);
    expect(session.loading).toBeNull();
    expect(error).not.toHaveBeenCalled();
  });

  it("an open that was never abandoned doesn't open the photo twice", async () => {
    await opened(session, 10);
    await opened(session, 11);
    expect(openImage.fn).toHaveBeenCalledTimes(2);
  });

  it("stays on the photo shown when another can't be opened", async () => {
    await opened(session, 1);
    const failing = session.open(2);
    openImage.calls.find((c) => c.args[0] === 2)!.answer.reject("unsupported camera");
    historyOpen.calls.find((c) => c.args[0] === 2)!.answer.resolve(checkout(2, 1));
    await failing;
    expect(session.image?.id).toBe(1);
    expect(error).toHaveBeenCalledWith("Couldn’t open this photo", "unsupported camera");
    // And the photo that failed can be tried again.
    session.open(2);
    expect(openImage.calls.filter((c) => c.args[0] === 2)).toHaveLength(2);
  });
});

describe("recording edits", () => {
  it("sends what is on screen, for the photo on screen", async () => {
    await opened(session, 1);
    session.edit!.light.exposure = 1;
    const done = session.commit("exposure");
    await settle();
    expect(commit.calls[0].args[0]).toBe(1);
    expect(commit.calls[0].args[1].light.exposure).toBe(1);
    expect(commit.calls[0].args[2]).toBe("exposure");
    commit.calls[0].answer.resolve(checkout(1, 2, 1));
    await done;
    expect(session.history?.head.commit).toBe(2);
    // The reply doesn't replace the edit: the user may already be dragging again.
    session.edit!.light.exposure = 1.5;
    expect(session.edit?.light.exposure).toBe(1.5);
  });

  it("a commit made behind an undo records what the undo shows, not what it replaced", async () => {
    await opened(session, 1, checkout(1, 2, 1));
    const undone = session.undo();
    // Before the undo has answered, something commits (a slider released late).
    const committed = session.commit(null);
    await settle();
    expect(commit.fn).not.toHaveBeenCalled(); // strictly in order
    undo.calls[0].answer.resolve(checkout(1, 1, 0));
    await undone;
    await settle();
    expect(session.edit?.light.exposure).toBe(0);
    expect(commit.calls[0].args[1].light.exposure).toBe(0);
    commit.calls[0].answer.resolve(checkout(1, 1, 0));
    await committed;
  });

  it("a reply for a photo that was left changes nothing on the one now shown", async () => {
    await opened(session, 1);
    session.edit!.light.exposure = 1;
    const late = session.commit("exposure");
    await settle();
    await opened(session, 2, checkout(2, 1, 0.25));
    commit.calls[0].answer.resolve(checkout(1, 2, 1));
    await late;
    expect(session.history?.imageId).toBe(2);
    expect(session.history?.head.commit).toBe(1);
    expect(session.edit?.light.exposure).toBe(0.25);
    expect(scheduleLibraryThumb).not.toHaveBeenCalledWith(1, expect.anything());
  });

  it("refreshes the library thumbnail when HEAD moves, and when its step changes in place", async () => {
    await opened(session, 1);
    // A new step.
    let done = session.commit("exposure");
    await settle();
    commit.calls[0].answer.resolve(checkout(1, 2, 0.1, 100));
    await done;
    expect(scheduleLibraryThumb).toHaveBeenCalledTimes(1);
    // The same control nudged again at once: merged into that step. Same id, later time.
    done = session.commit("exposure");
    await settle();
    commit.calls[1].answer.resolve(checkout(1, 2, 1, 900));
    await done;
    expect(scheduleLibraryThumb).toHaveBeenCalledTimes(2);
    expect(scheduleLibraryThumb.mock.calls[1][1].light.exposure).toBe(1);
    // A commit that changed nothing: the same step, untouched.
    done = session.commit("exposure");
    await settle();
    commit.calls[2].answer.resolve(checkout(1, 2, 1, 900));
    await done;
    expect(scheduleLibraryThumb).toHaveBeenCalledTimes(2);
  });

  it("an operation that fails doesn't hold up the ones after it", async () => {
    await opened(session, 1);
    const failed = session.commit("exposure");
    const next = session.commit("contrast");
    await settle();
    commit.calls[0].answer.reject("disk full");
    await expect(failed).rejects.toBe("disk full");
    expect(error).toHaveBeenCalledWith("Couldn’t update the history", "disk full");
    await settle();
    commit.calls[1].answer.resolve(checkout(1, 2));
    await next;
    expect(session.history?.head.commit).toBe(2);
  });
});
