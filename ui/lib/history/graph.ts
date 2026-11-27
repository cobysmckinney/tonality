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
  const ahead = new Set(
    headBranch ? lineage(parents, headBranch.tip).filter((id) => !behindHead.has(id)) : [],
  );

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
    edges.push({
      fromRow: r.row,
      fromLane: r.lane,
      toRow,
      toLane: laneOf.get(p) ?? 0,
      color: r.color,
    });
  }

  return { rows, edges, lanes: Math.max(lanes, 1), branchColor };
}

// ---------------------------------------------------------------------------------------------
// One version at a time: what the History panel shows.
//
// Each commit belongs to the earliest-created branch that reaches it (the same rule the lane
// graph uses), so a branch's own steps are the work done since it forked, and everything older
// is inherited from the branch it forked from.
// ---------------------------------------------------------------------------------------------

/** The branch that owns each commit. */
export function commitOwners(view: HistoryView): Map<number, string> {
  const parents = new Map(view.commits.map((c) => [c.id, c.parent]));
  const owner = new Map<number, string>();
  for (const b of view.branches) {
    for (const id of lineage(parents, b.tip)) {
      if (owner.has(id)) break;
      owner.set(id, b.name);
    }
  }
  return owner;
}

export interface Step {
  commit: CommitInfo;
  isHead: boolean;
  /** After HEAD on this branch: what redo would bring back. */
  ahead: boolean;
  /** Other versions that split off at this step. */
  forks: string[];
}

export interface StepGroup {
  /** The branch these steps belong to. */
  branch: string;
  /** Inherited from an ancestor version rather than made on this one. */
  inherited: boolean;
  steps: Step[];
}

/** The steps of `branch` (default: the current one), newest first, grouped by owning branch. */
export function branchSteps(view: HistoryView, branch = view.head.branch): StepGroup[] {
  const b = view.branches.find((x) => x.name === branch);
  if (!b) return [];
  const parents = new Map(view.commits.map((c) => [c.id, c.parent]));
  const byId = new Map(view.commits.map((c) => [c.id, c]));
  const owner = commitOwners(view);
  const line = lineage(parents, b.tip);
  const onLine = new Set(line);
  const isCurrent = branch === view.head.branch;
  const behindHead = isCurrent ? new Set(lineage(parents, view.head.commit)) : onLine;

  // Where each other version splits from this one (unless it's an ancestor shown as a group).
  const ancestors = new Set(line.map((id) => owner.get(id)));
  const forksAt = new Map<number, string[]>();
  for (const other of view.branches) {
    if (other.name === branch || ancestors.has(other.name)) continue;
    const at = lineage(parents, other.tip).find((id) => onLine.has(id));
    if (at !== undefined) forksAt.set(at, [...(forksAt.get(at) ?? []), other.name]);
  }

  const groups: StepGroup[] = [];
  for (const id of line) {
    const commit = byId.get(id);
    if (!commit) continue;
    const name = owner.get(id) ?? branch;
    let g = groups.at(-1);
    if (!g || g.branch !== name) {
      g = { branch: name, inherited: name !== branch, steps: [] };
      groups.push(g);
    }
    g.steps.push({
      commit,
      isHead: isCurrent && id === view.head.commit,
      ahead: !behindHead.has(id),
      forks: forksAt.get(id) ?? [],
    });
  }
  return groups;
}

/**
 * What deleting version `name` does: how many steps go with it (the ones no other version
 * reaches; steps it shares stay), and the version the photo switches to if it is the current
 * one. `null` when it can't be deleted (it's the only version, or isn't one).
 */
export function deletion(
  view: HistoryView,
  name: string,
): { steps: number; switchesTo: string | null } | null {
  const gone = view.branches.find((b) => b.name === name);
  const others = view.branches.filter((b) => b.name !== name);
  if (!gone || !others.length) return null;
  const parents = new Map(view.commits.map((c) => [c.id, c.parent]));
  const kept = new Set(others.flatMap((b) => lineage(parents, b.tip)));
  return {
    steps: lineage(parents, gone.tip).filter((id) => !kept.has(id)).length,
    // The catalog moves HEAD to the first version left, in creation order.
    switchesTo: view.head.branch === name ? others[0].name : null,
  };
}

export interface VersionInfo {
  name: string;
  tip: number;
  /** Steps made on this version since it split off. */
  own: number;
  /** The version it was branched from, if any. */
  parent: string | null;
  color: number;
}

/** Summary of every version, in creation order. */
export function versions(view: HistoryView): VersionInfo[] {
  const parents = new Map(view.commits.map((c) => [c.id, c.parent]));
  const owner = commitOwners(view);
  const colors = layoutGraph(view).branchColor;
  return view.branches.map((b) => {
    const line = lineage(parents, b.tip);
    const own = line.filter((id) => owner.get(id) === b.name).length;
    const firstForeign = line.find((id) => owner.get(id) !== b.name);
    return {
      name: b.name,
      tip: b.tip,
      own,
      parent: firstForeign !== undefined ? (owner.get(firstForeign) ?? null) : null,
      color: colors.get(b.name) ?? 0,
    };
  });
}
