// Type-checked with the app, so it fails to build when the backend gains a
// call the fake lacks; bun:test only has types under `bun test`.
// @ts-expect-error
import { mock } from "bun:test";
import { DEFAULTS, type Adjustments } from "./adjustments";
import type { api as realApi, History } from "./api";

/**
 * A stand-in for the backend, for tests that load the store.
 *
 * bun shares module mocks, and the store, between all the test files in a
 * run, so the store keeps whichever `./api` it was first given, and work it
 * left waiting in one file (a thumbnail redraw, say) can call the backend
 * while the next file runs. So every file uses this one fake: it has every
 * call, and each file says only how the calls it cares about answer:
 *
 *     fakeApi({ openEditor: async () => ... });
 *     const { useStore } = await import("./store");
 *
 * The rest answer the way an empty library would. Calls that would have to
 * make up a whole record (a preset, a scan, an export) fail instead, saying
 * the test didn't give one.
 */
export type Api = typeof realApi;

const history = (adjustments: Adjustments = DEFAULTS): History => ({
  branches: [],
  branchId: 1,
  steps: [],
  headId: 1,
  adjustments,
});
const notGiven = (call: string) => async (): Promise<never> => {
  throw `The test didn't say what ${call} answers`;
};
const done = { versions: [], cancelled: false, failed: [] };

const quiet = (): Api => ({
  libraryProblem: async () => null,
  retryLibrary: async () => {},
  getOverview: async () => ({
    libraryPath: "/library",
    photoCount: 0,
    favoriteCount: 0,
    deletedCount: 0,
    albums: [],
    imports: [],
  }),
  listPhotos: async () => [],
  getPhotoInfo: notGiven("getPhotoInfo"),

  setFavorite: async () => {},
  setFlag: async () => {},

  getFilmDetails: async () => [],
  setFilmDetails: async () => {},
  filmSuggestions: async () => ({ stocks: [], cameras: [], lenses: [] }),

  trashPhotos: async () => {},
  restorePhotos: async () => {},
  purgePhotos: async (ids) => ids.length,

  createAlbum: async () => 1,
  renameAlbum: async () => {},
  deleteAlbum: async () => {},
  addToAlbum: async () => {},
  removeFromAlbum: async () => {},

  listVolumes: async () => [],
  chooseImport: async () => [],
  choosePresetFiles: async () => [],
  choosePresetDestination: async () => null,
  chooseExportFolder: async () => null,
  scanImport: notGiven("scanImport"),
  discardScan: async () => {},
  runImport: async () => ({ imported: 0, restored: 0, failed: [], cancelled: false }),
  cancelImport: async () => {},

  openEditor: notGiven("openEditor"),
  closeEditor: async () => {},
  renderFrame: notGiven("renderFrame"),
  maskMattes: async () => [],
  findParts: async () => {},
  prepareCircles: async () => {},
  refreshRendered: async (ids) => ids.map(() => 1),
  applyEdits: async (ids) => ({ ...done, versions: ids.map(() => 1) }),
  filmBase: notGiven("filmBase"),
  pickFilmBase: notGiven("pickFilmBase"),
  balanceRoll: async (ids) => ({ ...done, versions: ids.map(() => 1) }),
  cancelEdits: async () => {},
  getEdits: async () => DEFAULTS,

  listPresets: async () => [],
  createPreset: notGiven("createPreset"),
  renamePreset: notGiven("renamePreset"),
  updatePreset: notGiven("updatePreset"),
  deletePreset: async () => {},
  favoritePresets: async () => [],
  setPresetFavorite: async () => [],
  presetPreviews: async () => new Map(),
  exportPreset: async () => {},
  importPresets: async () => ({ imported: [], failed: [] }),
  applyPreset: async (ids) => ({ ...done, versions: ids.map(() => 1) }),

  getHistory: async () => history(),
  historyCommit: async (_id, adjustments) => history(adjustments),
  historyGoto: async () => history(),
  historyBranch: async () => history(),
  historySwitch: async () => history(),
  historyRenameBranch: async () => history(),
  historyDeleteBranch: async () => history(),

  planExport: notGiven("planExport"),
  runExport: notGiven("runExport"),
  cancelExport: async () => {},
});

// The one object the store sees; `fakeApi` changes its calls in place.
const api: Api = quiet();
mock.module("./api", () => ({ api }));

/** Starts the backend afresh, with these calls answered as given. */
export function fakeApi(calls: Partial<Api> = {}) {
  Object.assign(api, quiet(), calls);
}
