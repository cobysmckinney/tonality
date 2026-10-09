/**
 * Healing in the interface: making spots, naming their steps, and the
 * arithmetic of drawing and moving them. Spots are kept on the photo file
 * (see Spot in adjustments.ts), like mask parts, so they stay on their speck
 * whatever the crop, turn or flip; `screenMap` in masks.ts places them on
 * screen. Mirrors the limits in src-tauri/src/heal.rs.
 */
import { Point, Spot } from "./adjustments";
import { tidy } from "./masks";

/** The most spots a photo can have. */
export const MAX_SPOTS = 500;
/** The most points one spot's line keeps. */
export const MAX_POINTS = 64;

/** A speck the backend found that looks like dust, and the patch that would cover it. Mirrors `Speck` in heal.rs. */
export interface Speck {
  center: Point;
  /** A share of the photo's longer side. */
  radius: number;
  source: Point;
}

const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

/** The size slider (1..100) as a radius, in shares of the photo's longer side. Finer than the mask brush: most dust is small. */
export const healRadius = (size: number) => 0.08 * (clamp(size, 1, 100) / 100) ** 2;

/** The size slider's value for a radius: `healRadius` the other way. */
export const sizeForRadius = (radius: number) => clamp(Math.round(100 * Math.sqrt(Math.max(0, radius) / 0.08)), 1, 100);

/** The default size: a speck of dust a few dozen pixels across on a full-size photo. */
export const HEAL_SIZE = 20;

const minus = (a: Point, b: Point): Point => [a[0] - b[0], a[1] - b[1]];
const plus = (a: Point, b: Point): Point => [a[0] + b[0], a[1] + b[1]];

/** How far `q` is from the segment from `a` to `b`. */
function fromSegment(q: Point, a: Point, b: Point): number {
  const ab = minus(b, a);
  const length = ab[0] * ab[0] + ab[1] * ab[1];
  const t = length > 0 ? clamp(((q[0] - a[0]) * ab[0] + (q[1] - a[1]) * ab[1]) / length, 0, 1) : 0;
  return Math.hypot(q[0] - a[0] - ab[0] * t, q[1] - a[1] - ab[1] * t);
}

/**
 * A line drawn by hand, kept short: the points that matter to its shape
 * within `tolerance` (Ramer–Douglas–Peucker), at most `limit`, spread evenly
 * if there are still too many.
 */
export function simplify(points: Point[], tolerance: number, limit = MAX_POINTS): Point[] {
  if (points.length <= 2) return points;
  const keep = new Array<boolean>(points.length).fill(false);
  keep[0] = keep[points.length - 1] = true;
  const stack: [number, number][] = [[0, points.length - 1]];
  while (stack.length) {
    const [from, to] = stack.pop()!;
    let far = -1;
    let farthest = tolerance;
    for (let i = from + 1; i < to; i++) {
      const d = fromSegment(points[i], points[from], points[to]);
      if (d > farthest) [far, farthest] = [i, d];
    }
    if (far >= 0) {
      keep[far] = true;
      stack.push([from, far], [far, to]);
    }
  }
  const kept = points.filter((_, i) => keep[i]);
  if (kept.length <= limit) return kept;
  const last = kept.length - 1;
  return Array.from({ length: limit }, (_, i) => kept[Math.round((i * last) / (limit - 1))]);
}

/** Where the patch covers: the spot's line, moved to its patch. */
export const patchPoints = (spot: Spot): Point[] => {
  const offset = minus(spot.source, spot.points[0]);
  return spot.points.map((p) => plus(p, offset));
};

/** The spot moved by `by` on the file; its patch stays where it is. */
export const moveSpot = (spot: Spot, by: Point): Spot => ({
  ...spot,
  points: spot.points.map((p) => plus(p, by).map(tidy) as Point),
});

/** The spot with its patch moved by `by` on the file. */
export const movePatch = (spot: Spot, by: Point): Spot => ({ ...spot, source: plus(spot.source, by).map(tidy) as Point });

/** The spots that would cover these specks. */
export const specksAsSpots = (specks: Speck[]): Spot[] =>
  specks.map((speck) => ({ points: [speck.center], radius: speck.radius, source: speck.source }));

/** What the history calls each change to the spots. */
export const HEAL_STEPS = {
  added: (spot: Spot) => (spot.points.length > 1 ? "Healed a line" : "Healed a spot"),
  moved: "Moved a spot",
  patch: "Moved a patch",
  resized: "Resized a spot",
  deleted: "Deleted a spot",
  cleared: "Deleted all spots",
  specks: (count: number) => (count === 1 ? "Removed a speck" : `Removed ${count} specks`),
};

/** An SVG path through screen points; a single point is a dot once drawn with round caps. */
export const pathThrough = (points: Point[]) =>
  points.length === 0 ? "" : `M${points.map((p) => `${p[0]} ${p[1]}`).join("L")}${points.length === 1 ? `L${points[0][0]} ${points[0][1]}` : ""}`;

/** How far a line drawn on screen runs, end to end along its points. */
export const travel = (points: Point[]) =>
  points.slice(1).reduce((sum, p, i) => sum + Math.hypot(p[0] - points[i][0], p[1] - points[i][1]), 0);
