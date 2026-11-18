import { describe, expect, it } from "vitest";
import type { HistoryView } from "$lib/bindings/HistoryView";
import { branchSteps, layoutGraph, relativeTime, versions } from "./graph";

const c = (id: number, parent: number | null) => ({ id, parent, summary: `c${id}`, time: 0 });

// main: 1 ← 2 ← 3 ; bw forks at 2: 2 ← 4 ← 5 ; hdr sits on 1 with no commits of its own.
const view: HistoryView = {
  imageId: 1,
  commits: [c(1, null), c(2, 1), c(3, 2), c(4, 2), c(5, 4)],
  branches: [
    { name: "main", tip: 3 },
    { name: "bw", tip: 5 },
    { name: "hdr", tip: 1 },
  ],
  head: { branch: "bw", commit: 4 },
  canUndo: true,
  canRedo: true,
};

describe("layoutGraph", () => {
  const g = layoutGraph(view);
  const row = (id: number) => g.rows.find((r) => r.commit.id === id)!;

  it("orders newest first", () => {
    expect(g.rows.map((r) => r.commit.id)).toEqual([5, 4, 3, 2, 1]);
  });

  it("gives main the shared trunk and later branches their own work", () => {
    expect([1, 2, 3].map((id) => row(id).lane)).toEqual([0, 0, 0]);
    expect([4, 5].map((id) => row(id).lane)).toEqual([1, 1]);
    expect(g.lanes).toBe(2);
  });

  it("colours branches by lane, borrowing for lane-less branches", () => {
    expect(g.branchColor.get("main")).toBe(0);
    expect(g.branchColor.get("bw")).toBe(1);
    expect(g.branchColor.get("hdr")).toBe(0);
    expect(row(1).tips.map((t) => t.branch.name)).toEqual(["hdr"]);
  });

  it("marks head and redo-able commits", () => {
    expect(row(4).isHead).toBe(true);
    expect(row(5).ahead).toBe(true);
    expect(row(3).ahead).toBe(false);
  });

  it("draws the fork edge from the branch lane into the trunk", () => {
    const e = g.edges.find((e) => e.fromRow === row(4).row)!;
    expect(e).toMatchObject({ fromLane: 1, toLane: 0, toRow: row(2).row, color: 1 });
  });
});

describe("relativeTime", () => {
  it("formats", () => {
    const now = 1_000_000_000;
    expect(relativeTime(now - 10_000, now)).toBe("now");
    expect(relativeTime(now - 5 * 60_000, now)).toBe("5m");
    expect(relativeTime(now - 3 * 3_600_000, now)).toBe("3h");
  });
});

describe("branchSteps", () => {
  const ids = (groups: ReturnType<typeof branchSteps>) =>
    groups.map((g) => [g.branch, g.inherited, g.steps.map((s) => s.commit.id)]);

  it("shows a forked version's own work, then what it inherited", () => {
    expect(ids(branchSteps(view))).toEqual([
      ["bw", false, [5, 4]],
      ["main", true, [2, 1]],
    ]);
  });

  it("marks HEAD and the redo-able steps", () => {
    const [own] = branchSteps(view);
    expect(own.steps.map((s) => [s.commit.id, s.isHead, s.ahead])).toEqual([
      [5, false, true],
      [4, true, false],
    ]);
  });

  it("marks where other versions split off, not the parent", () => {
    const main = branchSteps(view, "main");
    expect(ids(main)).toEqual([["main", false, [3, 2, 1]]]);
    const forks = Object.fromEntries(main[0].steps.map((s) => [s.commit.id, s.forks]));
    expect(forks).toEqual({ 3: [], 2: ["bw"], 1: ["hdr"] });
    // From bw, main is the ancestor (a group), and hdr splits at 1.
    expect(
      branchSteps(view)
        .at(-1)!
        .steps.find((s) => s.commit.id === 1)!.forks,
    ).toEqual(["hdr"]);
  });

  it("gives a fresh version no steps of its own", () => {
    expect(ids(branchSteps(view, "hdr"))).toEqual([["main", true, [1]]]);
  });
});

describe("versions", () => {
  it("counts own steps and names the parent", () => {
    expect(versions(view).map((v) => [v.name, v.own, v.parent])).toEqual([
      ["main", 3, null],
      ["bw", 2, "main"],
      ["hdr", 0, "main"],
    ]);
  });
});
