import { describe, expect, it } from "vitest";
import type { HistoryView } from "$lib/bindings/HistoryView";
import { layoutGraph, relativeTime } from "./graph";

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
