import { expect, mock, test } from "bun:test";

// A backend whose presets fail to load.
const card = { name: "EOS_DIGITAL", path: "/media/card" };
mock.module("./api", () => ({
  api: {
    libraryProblem: async () => null,
    getOverview: async () => ({ total: 0, imports: [], albums: [] }),
    listPhotos: async () => [],
    listVolumes: async () => [card],
    listPresets: async () => {
      throw "database is locked";
    },
    favoritePresets: async () => [3],
  },
}));
const { useStore } = await import("./store");

test("when one thing fails to load at startup, it's reported and the rest still loads", async () => {
  useStore.setState({ toasts: [] });
  await useStore.getState().init();
  const state = useStore.getState();
  expect(state.loaded).toBe(true);
  expect(state.volumes).toEqual([card]);
  expect(state.favoritePresets).toEqual([3]);
  expect(state.toasts.map((t) => [t.text, t.tone])).toEqual([["database is locked", "error"]]);
});
