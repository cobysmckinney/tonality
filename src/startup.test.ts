import { expect, mock, test } from "bun:test";

// A backend whose presets fail to load.
const card = { name: "EOS_DIGITAL", path: "/media/card" };
const camera = { name: "Canon EOS 600D", port: "usb:001,005" };
mock.module("./api", () => ({
  api: {
    libraryProblem: async () => null,
    getOverview: async () => ({ total: 0, imports: [], albums: [] }),
    listPhotos: async () => [],
    listVolumes: async () => [card],
    listCameras: async () => [camera],
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
  expect(state.cameras).toEqual([camera]);
  expect(state.favoritePresets).toEqual([3]);
  expect(state.toasts.map((t) => [t.text, t.tone])).toEqual([["database is locked", "error"]]);
});
