import { beforeEach, expect, mock, test } from "bun:test";
import { DEFAULTS, Mask } from "./adjustments";
import { newMask } from "./masks";

// The backend, with finding parts held until a test lets it finish.
let finishFinding: () => void = () => {};
mock.module("./api", () => ({
  api: {
    findParts: () => new Promise<void>((resolve) => (finishFinding = resolve)),
    prepareCircles: async () => {},
    historyCommit: async (_id: number, adjustments: unknown) => ({ branches: [], branchId: 1, steps: [], headId: 1, adjustments }),
    refreshRendered: async () => [1],
  },
}));
const { useStore } = await import("./store");

const size = { width: 300, height: 200 };
const state = () => useStore.getState();
const masks = () => state().editor.adjustments.masks;
/** Lets the store's waiting code run. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

let a: Mask;
let b: Mask;
beforeEach(() => {
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
  finishFinding();
  await settle();
  expect(masks()[0].parts.map((p) => p.shape.kind)).toEqual(["radial", "subject"]);
  expect(masks()[1]).toEqual(b);
  expect(state().editor.maskId).toBe(b.id);
});

test("a found part is still added when no mask is chosen meanwhile", async () => {
  state().addMaskPart("sky", "subtract");
  state().selectMask(null);
  finishFinding();
  await settle();
  expect(masks()[0].parts.at(-1)).toEqual({ mode: "subtract", shape: { kind: "sky" } });
});

test("a found part for a mask deleted meanwhile says so", async () => {
  state().addMaskPart("subject", "add");
  state().removeMask(a.id);
  finishFinding();
  await settle();
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
  finishFinding();
  await settle();
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
  finishFinding();
  await settle();
  expect(masks()[0].parts).toEqual(a.parts);
  expect(state().toasts.map((t) => t.text)).toEqual(["The part was removed before the object was found"]);
});
