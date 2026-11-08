// Lays out a HistoryView as a git-style lane graph: newest commit on top, one lane per branch.
//
// Lanes are claimed in branch creation order: `main` walks back from its tip and takes every
// commit it reaches; each later branch takes only the commits no earlier branch claimed (its own
// work since forking). A branch with no commits of its own gets no lane.
import type { BranchInfo } from "$lib/bindings/BranchInfo";
import type { CommitInfo } from "$lib/bindings/CommitInfo";
import type { HistoryView } from "$lib/bindings/HistoryView";

export const BRANCH_COLORS = 6;

export interface GraphRow {
  commit: CommitInfo;
  row: number;
  lane: number;
  color: number;
  /** Branches whose tip is this commit. */
  tips: { branch: BranchInfo; color: number }[];
  isHead: boolean;
  /** On the current branch but after HEAD (i.e. what redo would bring back). */
  ahead: boolean;
}

export interface GraphEdge {
  fromRow: number;
  fromLane: number;
  toRow: number;
  toLane: number;
  color: number;
}

export interface GraphLayout {
  rows: GraphRow[];
  edges: GraphEdge[];
  lanes: number;
  /** Colour index of each branch, by name. */
  branchColor: Map<string, number>;
}

function lineage(parents: Map<number, number | null>, tip: number): number[] {
  const out: number[] = [];
  for (let c: number | null | undefined = tip; c != null; c = parents.get(c)) out.push(c);
  return out;
}

export function layoutGraph(view: HistoryView): GraphLayout {
  const parents = new Map(view.commits.map((c) => [c.id, c.parent]));
  const laneOf = new Map<number, number>();
  const branchColor = new Map<string, number>();
  let lanes = 0;

  for (const b of view.branches) {
    let claimed = false;
    for (const id of lineage(parents, b.tip)) {
      if (laneOf.has(id)) break;
      laneOf.set(id, lanes);
      claimed = true;
    }
    if (claimed) lanes++;
  }
  const color = (lane: number) => lane % BRANCH_COLORS;
  for (const b of view.branches) branchColor.set(b.name, color(laneOf.get(b.tip) ?? 0));
  // A branch with no commits of its own borrows the colour of the lane its tip sits on.

  const headBranch = view.branches.find((b) => b.name === view.head.branch);
  const behindHead = new Set(lineage(parents, view.head.commit));
  const ahead = new Set(headBranch ? lineage(parents, headBranch.tip).filter((id) => !behindHead.has(id)) : []);

  const ordered = [...view.commits].sort((a, b) => b.id - a.id);
  const rowOf = new Map(ordered.map((c, i) => [c.id, i]));
  const rows: GraphRow[] = ordered.map((commit, row) => {
    const lane = laneOf.get(commit.id) ?? 0;
    return {
      commit,
      row,
      lane,
      color: color(lane),
      tips: view.branches
        .filter((b) => b.tip === commit.id)
        .map((branch) => ({ branch, color: branchColor.get(branch.name) ?? 0 })),
      isHead: commit.id === view.head.commit,
      ahead: ahead.has(commit.id),
    };
  });

  const edges: GraphEdge[] = [];
  for (const r of rows) {
    const p = r.commit.parent;
    if (p == null) continue;
    const toRow = rowOf.get(p);
    if (toRow === undefined) continue;
    edges.push({ fromRow: r.row, fromLane: r.lane, toRow, toLane: laneOf.get(p) ?? 0, color: r.color });
  }

  return { rows, edges, lanes: Math.max(lanes, 1), branchColor };
}

/** Compact relative time: "now", "5m", "3h", "2d", or a date. */
export function relativeTime(ms: number, now = Date.now()): string {
  const s = Math.max(0, (now - ms) / 1000);
  if (s < 45) return "now";
  if (s < 3600) return `${Math.round(s / 60)}m`;
  if (s < 86400) return `${Math.round(s / 3600)}h`;
  if (s < 7 * 86400) return `${Math.round(s / 86400)}d`;
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}
