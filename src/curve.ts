/**
 * Editing a tone curve's points: dragging, removing, and the keyboard's
 * equivalents. Points run from [0, 0] (black) to [1, 1] (white), in order.
 */
import { curveSampler, CurvePoints, POINT_GAP } from "./adjustments";

const clamp = (v: number, low = 0, high = 1) => Math.min(high, Math.max(low, v));
/** How far an arrow key moves a point, as a share of the curve's width: 1%, or 5% with Shift. */
export const CURVE_STEP = 0.01;
export const BIG_CURVE_STEP = 0.05;

/** The points with one moved to `[x, y]`: it stays between its neighbours, and the ends keep their place across. */
export function movePoint(points: CurvePoints, index: number, [x, y]: [number, number]): CurvePoints {
  const last = points.length - 1;
  const fixedX = index === 0 ? 0 : index === last ? 1 : clamp(x, points[index - 1][0] + POINT_GAP, points[index + 1][0] - POINT_GAP);
  return points.map((p, i) => (i === index ? [fixedX, clamp(y)] : p));
}

/** The points without one. The ends can't go; they go back to their corner instead. */
export function removePoint(points: CurvePoints, index: number): CurvePoints {
  const last = points.length - 1;
  if (index === 0 || index === last) return points.map((p, i) => (i === index ? [p[0], index === 0 ? 0 : 1] : p));
  return points.filter((_, i) => i !== index);
}

/** Where an arrow key moves a point: Up and Down change its output, Left and Right its input. Null for other keys. */
export function nudgePoint(points: CurvePoints, index: number, key: string, big: boolean): CurvePoints | null {
  const step = big ? BIG_CURVE_STEP : CURVE_STEP;
  const [x, y] = points[index];
  const round = (v: number) => Math.round(v * 1000) / 1000;
  const moves: Record<string, [number, number]> = {
    ArrowUp: [x, y + step],
    ArrowDown: [x, y - step],
    ArrowLeft: [x - step, y],
    ArrowRight: [x + step, y],
  };
  const to = moves[key];
  return to ? movePoint(points, index, [round(to[0]), round(to[1])]) : null;
}

/**
 * The points with a new one on the curve, halfway between point `index` and
 * the next (or the one before, from the last point), and where it went in.
 * Null when they are too close together to fit one in.
 */
export function addPointNear(points: CurvePoints, index: number): { points: CurvePoints; index: number } | null {
  const after = index < points.length - 1 ? index : index - 1;
  const [a, b] = [points[after], points[after + 1]];
  if (!a || !b || b[0] - a[0] < 2 * POINT_GAP) return null;
  const x = Math.round(((a[0] + b[0]) / 2) * 1000) / 1000;
  const y = Math.round(curveSampler(points)(x) * 1000) / 1000;
  const next: CurvePoints = [...points.slice(0, after + 1), [x, y], ...points.slice(after + 1)];
  return { points: next, index: after + 1 };
}
