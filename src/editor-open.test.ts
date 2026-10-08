import { expect, mock, test } from "bun:test";

// The backend, with opening a photo answered by whatever a test sets.
let openEditor: (id: number) => Promise<unknown> = async () => {
  throw new Error("not set");
};
mock.module("./api", () => ({
  api: {
    openEditor: (id: number) => openEditor(id),
    closeEditor: async () => {},
  },
}));
const { useStore } = await import("./store");

const state = () => useStore.getState();

test("a photo that can't be opened says why in the editor, not in a toast", async () => {
  openEditor = async () => {
    throw "The original file is missing from the library folder (2024/IMG_1.CR2). Put it back there to edit or export this photo.";
  };
  useStore.setState({ toasts: [] });
  state().openPhoto(7);
  await state().loadEditor(7);
  expect(state().editor.photoId).toBe(7);
  expect(state().editor.ready).toBe(false);
  expect(state().editor.failed).toContain("The original file is missing");
  expect(state().toasts).toEqual([]);
});

test("opening the photo again starts fresh, and works once the file is back", async () => {
  openEditor = async () => ({ width: 30, height: 20, history: { branches: [], branchId: 1, steps: [], headId: 1, adjustments: state().editor.adjustments }, missing: [] });
  await state().loadEditor(7);
  expect(state().editor.ready).toBe(true);
  expect(state().editor.failed).toBeNull();
});

test("a failure for a photo already left behind is not pinned on the next one", async () => {
  let fail: () => void = () => {};
  openEditor = () => new Promise((_, reject) => (fail = () => reject("damaged")));
  state().openPhoto(8);
  const loading = state().loadEditor(8);
  await new Promise((resolve) => setTimeout(resolve, 0));
  state().openPhoto(9);
  fail();
  await loading;
  expect(state().editor.failed).toBeNull();
});
