import { beforeEach, expect, mock, test } from "bun:test";
import { DEFAULTS, Mask } from "./adjustments";
import { newMask } from "./masks";

// The backend, with each find held until a test lets it finish.
const finds: { kinds: string[]; finish: () => void }[] = [];
const steps: string[] = [];
mock.module("./api", () => ({
  api: {
    findParts: (_id: number, shapes: { kind: string }[]) =>
      new Promise<void>((finish) => finds.push({ kinds: shapes.map((shape) => shape.kind), finish })),
    prepareCircles: async () => {},
    historyCommit: async (_id: number, adjustments: unknown, name: string) => {
      steps.push(name);
      return { branches: [], branchId: 1, steps: [], headId: 1, adjustments };
    },
    refreshRendered: async () => [1],
  },
}));
const { useStore } = await import("./store");

const size = { width: 300, height: 200 };
const state = () => useStore.getState();
const masks = () => state().editor.adjustments.masks;
/** Lets the store's waiting code run. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));
/** Lets the oldest find that has started finish. */
const finishFinding = async () => {
  await settle();
  const find = finds.shift();
  if (!find) throw new Error("nothing is being found");
  find.finish();
  await settle();
};

let a: Mask;
let b: Mask;
beforeEach(() => {
  finds.length = 0;
  steps.length = 0;
  a = newMask([], "radial", size, DEFAULTS);
  b = newMask([a], "linear", size, DEFAULTS);
  const adjustments = { ...DEFAULTS, masks: [a, b] };
  useStore.setState((s) => ({
    toasts: [],
    editor: { ...s.editor, photoId: 1, ready: true, size, adjustments, committed: adjustments, maskId: a.id, partIndex: 0, finding: null, circling: null },
  }));
});

test("a found part goes to the mask it was asked for, even if another is chosen meanwhile", async () => {
  state().addMaskPart("subject", "add");
  state().selectMask(b.id);
  await finishFinding();
  expect(masks()[0].parts.map((p) => p.shape.kind)).toEqual(["radial", "subject"]);
  expect(masks()[1]).toEqual(b);
  expect(state().editor.maskId).toBe(b.id);
});

test("a found part is still added when no mask is chosen meanwhile", async () => {
  state().addMaskPart("sky", "subtract");
  state().selectMask(null);
  await finishFinding();
  expect(masks()[0].parts.at(-1)).toEqual({ mode: "subtract", shape: { kind: "sky" } });
});

test("a found part for a mask deleted meanwhile says so", async () => {
  state().addMaskPart("subject", "add");
  state().removeMask(a.id);
  await finishFinding();
  expect(masks()).toEqual([b]);
  expect(state().toasts.map((t) => t.text)).toEqual(["The mask was deleted before the subject was found"]);
});

test("drawing an object again changes that part, even if another mask is chosen meanwhile", async () => {
  const old = { mode: "add" as const, shape: { kind: "object" as const, points: [[0, 0], [1, 0], [0, 1]] as [number, number][] } };
  const withObject = { ...a, parts: [old, ...a.parts] };
  useStore.setState((s) => ({
    editor: { ...s.editor, adjustments: { ...s.editor.adjustments, masks: [withObject, b] } },
  }));
  state().startCircle({ mode: "add", replace: 0 });
  state().finishCircle([[0.2, 0.2], [0.8, 0.2], [0.5, 0.8]]);
  // Changing the part's mode while it's found is kept.
  state().updateMask(a.id, { parts: [{ ...old, mode: "subtract" }, ...a.parts] }, "mode");
  state().selectMask(b.id);
  await finishFinding();
  expect(masks()[0].parts[0]).toEqual({ mode: "subtract", shape: { kind: "object", points: [[0.2, 0.2], [0.8, 0.2], [0.5, 0.8]] } });
  expect(masks()[0].parts.slice(1)).toEqual(a.parts);
  expect(masks()[1]).toEqual(b);
});

test("drawing an object again on a part removed meanwhile says so", async () => {
  const old = { mode: "add" as const, shape: { kind: "object" as const, points: [[0, 0], [1, 0], [0, 1]] as [number, number][] } };
  useStore.setState((s) => ({
    editor: { ...s.editor, adjustments: { ...s.editor.adjustments, masks: [{ ...a, parts: [old, ...a.parts] }, b] } },
  }));
  state().startCircle({ mode: "add", replace: 0 });
  state().finishCircle([[0.2, 0.2], [0.8, 0.2], [0.5, 0.8]]);
  state().removeMaskPart(0);
  await finishFinding();
  expect(masks()[0].parts).toEqual(a.parts);
  expect(state().toasts.map((t) => t.text)).toEqual(["The part was removed before the object was found"]);
});

test("edits pasted while a part is being found wait their turn", async () => {
  const sky = newMask([], "sky", size, DEFAULTS);
  useStore.setState({ clipboard: { ...DEFAULTS, exposure: 1, masks: [sky] } });
  state().addMaskPart("subject", "add");
  void state().pasteEdits([1]);
  await settle();
  expect(finds.map((f) => f.kinds)).toEqual([["subject"]]);
  await finishFinding();
  expect(steps).toEqual([`${a.name}: add subject`]);
  // The paste's find starts only once the first has ended.
  expect(finds.map((f) => f.kinds)).toEqual([["sky"]]);
  expect(state().editor.finding).toBe("the sky");
  await finishFinding();
  expect(steps).toEqual([`${a.name}: add subject`, "Paste edits"]);
  expect(state().editor.adjustments.exposure).toBe(1);
  expect(masks()).toEqual([sky]);
  expect(state().editor.finding).toBeNull();
});

test("a loop drawn again while a part is being found waits its turn, then changes the part", async () => {
  const old = { mode: "add" as const, shape: { kind: "object" as const, points: [[0, 0], [1, 0], [0, 1]] as [number, number][] } };
  useStore.setState((s) => ({
    editor: { ...s.editor, adjustments: { ...s.editor.adjustments, masks: [{ ...a, parts: [old, ...a.parts] }, b] } },
  }));
  state().addMaskPart("sky", "subtract");
  state().startCircle({ mode: "add", replace: 0 });
  state().finishCircle([[0.2, 0.2], [0.8, 0.2], [0.5, 0.8]]);
  await finishFinding();
  // The loop stays on the photo while its own object is found.
  expect(state().editor.circling?.points).toEqual([[0.2, 0.2], [0.8, 0.2], [0.5, 0.8]]);
  expect(finds.map((f) => f.kinds)).toEqual([["object"]]);
  await finishFinding();
  expect(masks()[0].parts[0].shape).toEqual({ kind: "object", points: [[0.2, 0.2], [0.8, 0.2], [0.5, 0.8]] });
  expect(masks()[0].parts.at(-1)).toEqual({ mode: "subtract", shape: { kind: "sky" } });
  expect(state().editor.circling).toBeNull();
  expect(state().editor.finding).toBeNull();
});

test("a loop still being drawn when another find ends is kept", async () => {
  state().addMaskPart("subject", "add");
  state().startCircle({ mode: "add" });
  await finishFinding();
  expect(state().editor.circling).toEqual({ mode: "add" });
});

/** Opens the crop tool and moves the crop twice, each a change of its own as a drag would make. */
const cropTwice = () => {
  useStore.setState({ sidePanel: "adjust", cropSteps: null });
  state().setSidePanel("crop");
  state().adjust({ crop: { x: 0.4, y: 0.5, width: 0.5, height: 0.5 } });
  state().markCrop();
  state().adjust({ crop: { x: 0.3, y: 0.5, width: 0.5, height: 0.5 } });
  state().markCrop();
};
const crop = () => state().editor.adjustments.crop;

test("cancelling the crop tool puts the crop back and leaves no step", async () => {
  cropTwice();
  state().cancelCrop();
  await settle();
  expect(crop()).toEqual(DEFAULTS.crop);
  // Nothing is left over to undo.
  expect(state().editor.adjustments).toBe(state().editor.committed);
  expect(state().sidePanel).toBe("adjust");
  expect(steps).toEqual([]);
});

test("leaving the crop tool keeps everything done in it as one step", async () => {
  cropTwice();
  state().setSidePanel("adjust");
  await settle();
  expect(crop().x).toBe(0.3);
  expect(steps).toEqual(["Crop"]);
});

test("undo in the crop tool takes back one change at a time, outside the history", async () => {
  cropTwice();
  await state().undo();
  expect(crop().x).toBe(0.4);
  await state().redo();
  expect(crop().x).toBe(0.3);
  await state().undo();
  await state().undo();
  expect(crop()).toEqual(DEFAULTS.crop);
  await settle();
  expect(steps).toEqual([]);
  // Done after undoing everything is no change at all.
  state().setSidePanel("adjust");
  await settle();
  expect(steps).toEqual([]);
});
