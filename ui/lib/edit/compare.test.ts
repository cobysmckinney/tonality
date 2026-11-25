import { describe, expect, it } from "vitest";
import type { EditState } from "$lib/bindings/EditState";
import type { HistoryView } from "$lib/bindings/HistoryView";
import type { ImageInfo } from "$lib/bindings/ImageInfo";
import {
  afterHistory,
  afterOpen,
  afterRemoved,
  compareLabels,
  type PhotoCompare,
  type VersionCompare,
} from "./compare";

const state = {} as EditState;
const version: VersionCompare = { kind: "version", branch: "bw", state };
const photo: PhotoCompare = {
  kind: "photo",
  image: { id: 7, name: "P7.CR3" } as ImageInfo,
  pixels: new Uint16Array(3),
  version: "main",
  state,
};

function view(branches: string[], head: string): HistoryView {
  return {
    imageId: 1,
    commits: [],
    branches: branches.map((name, i) => ({ name, tip: i })),
    head: { branch: head, commit: 0 },
    canUndo: false,
    canRedo: false,
  } as HistoryView;
}

describe("afterOpen", () => {
  it("drops a version compare: it belongs to the photo it came from", () => {
    expect(afterOpen(version, 2)).toBeNull();
  });
  it("keeps the reference photo across photo switches", () => {
    expect(afterOpen(photo, 2)).toBe(photo);
  });
  it("closes when the reference itself is opened", () => {
    expect(afterOpen(photo, 7)).toBeNull();
  });
  it("stays closed", () => {
    expect(afterOpen(null, 2)).toBeNull();
  });
});

describe("afterHistory", () => {
  it("keeps a version that still exists and isn't being edited", () => {
    expect(afterHistory(version, view(["main", "bw"], "main"))).toBe(version);
  });
  it("stops comparing a deleted version", () => {
    expect(afterHistory(version, view(["main"], "main"))).toBeNull();
  });
  it("stops comparing the version that's now being edited", () => {
    expect(afterHistory(version, view(["main", "bw"], "bw"))).toBeNull();
  });
  it("leaves a reference photo alone", () => {
    expect(afterHistory(photo, view(["main"], "main"))).toBe(photo);
  });
});

describe("afterRemoved", () => {
  it("closes when the reference is removed", () => {
    expect(afterRemoved(photo, [3, 7])).toBeNull();
  });
  it("keeps it when other photos are removed", () => {
    expect(afterRemoved(photo, [3])).toBe(photo);
    expect(afterRemoved(version, [7])).toBe(version);
  });
});

describe("compareLabels", () => {
  it("names versions as before", () => {
    expect(compareLabels(version, "main", "P1.CR3")).toEqual({ main: "main", other: "bw" });
  });
  it("names the reference by file and version, the edited photo by file", () => {
    expect(compareLabels(photo, "main", "P1.CR3")).toEqual({
      main: "P1.CR3",
      other: "P7.CR3 · main",
    });
  });
  it("has no branch on a reference, so version rows don't light up for it", () => {
    const c: VersionCompare | PhotoCompare = photo;
    expect(c.branch).toBeUndefined();
  });
});
