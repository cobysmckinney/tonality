// Crop-tool geometry (UI side). The renderer and export get framing from Rust
// (tonality_core::geometry); this module only answers interactive questions: does a crop
// rectangle fit inside the straightened image, and how to adjust one so it does.
import type { Crop } from "$lib/bindings/Crop";

export type Rect = [number, number, number, number]; // x, y, w, h — fractions of the turned frame

export const FULL: Rect = [0, 0, 1, 1];

/** Frame size after quarter turns. */
export function turnedSize(crop: Crop, w: number, h: number): [number, number] {
  return crop.turns % 2 ? [h, w] : [w, h];
}

/**
 * Does `rect` lie inside the image once straightened by `angle`° in a `tw × th` frame?
 * (Each corner, rotated back into the image, must land inside it.)
 */
export function fits(rect: Rect, angle: number, tw: number, th: number, eps = 1e-4): boolean {
  const a = (-angle * Math.PI) / 180;
  const c = Math.cos(a), s = Math.sin(a);
  const [x, y, w, h] = rect;
  if (w <= 0 || h <= 0 || x < -eps || y < -eps || x + w > 1 + eps || y + h > 1 + eps) return false;
  for (const [fx, fy] of [[x, y], [x + w, y], [x, y + h], [x + w, y + h]]) {
    const dx = (fx - 0.5) * tw, dy = (fy - 0.5) * th;
    const sx = (tw / 2 + c * dx - s * dy) / tw;
    const sy = (th / 2 + s * dx + c * dy) / th;
    if (sx < -eps || sx > 1 + eps || sy < -eps || sy > 1 + eps) return false;
  }
  return true;
}

/** Scale `rect` about its centre (keeping its shape) until it fits; also pulls it inside the frame. */
export function shrinkToFit(rect: Rect, angle: number, tw: number, th: number): Rect {
  const [x, y, w, h] = rect;
  const cx = Math.min(1, Math.max(0, x + w / 2)), cy = Math.min(1, Math.max(0, y + h / 2));
  const at = (k: number): Rect => [cx - (w * k) / 2, cy - (h * k) / 2, w * k, h * k];
  if (fits(at(1), angle, tw, th)) return at(1);
  let lo = 0, hi = 1;
  for (let i = 0; i < 30; i++) {
    const mid = (lo + hi) / 2;
    if (fits(at(mid), angle, tw, th)) lo = mid;
    else hi = mid;
  }
  return at(lo);
}

/** The furthest point from `from` (which fits) towards `to` that still fits. */
export function towards(from: Rect, to: Rect, angle: number, tw: number, th: number): Rect {
  if (fits(to, angle, tw, th)) return to;
  const lerp = (t: number) => from.map((v, i) => v + (to[i] - v) * t) as Rect;
  let lo = 0, hi = 1;
  for (let i = 0; i < 24; i++) {
    const mid = (lo + hi) / 2;
    if (fits(lerp(mid), angle, tw, th)) lo = mid;
    else hi = mid;
  }
  return lerp(lo);
}

/** The largest rect of pixel aspect `ratio` (w/h), centred on `rect`'s centre, that fits. */
export function withAspect(rect: Rect, ratio: number, angle: number, tw: number, th: number): Rect {
  const [x, y, w, h] = rect;
  const cx = x + w / 2, cy = y + h / 2;
  // Pick the shape (in frame fractions) for the ratio, as large as the current area allows.
  let nw = w, nh = (w * tw) / ratio / th;
  if (nh > 1) {
    nh = 1;
    nw = (ratio * th) / tw;
  }
  const big: Rect = [cx - nw / 2, cy - nh / 2, nw, nh];
  return shrinkToFit(clampInFrame(big), angle, tw, th);
}

/** Move a rect so it lies within the frame (it must already be no larger than the frame). */
export function clampInFrame([x, y, w, h]: Rect): Rect {
  w = Math.min(1, w);
  h = Math.min(1, h);
  return [Math.min(1 - w, Math.max(0, x)), Math.min(1 - h, Math.max(0, y)), w, h];
}

/** Turn the image a quarter turn (dir 1 = clockwise), carrying the crop along. */
export function turn(crop: Crop, dir: 1 | -1): Crop {
  const [x, y, w, h] = crop.rect as Rect;
  const rect: Rect = dir === 1 ? [1 - y - h, x, h, w] : [y, 1 - x - w, h, w];
  return { ...crop, turns: (crop.turns + (dir === 1 ? 1 : 3)) % 4, rect };
}

/** Mirror left-right, carrying the crop and straighten angle along. */
export function flip(crop: Crop): Crop {
  const [x, y, w, h] = crop.rect as Rect;
  return { ...crop, flip: !crop.flip, angle: -crop.angle, rect: [1 - x - w, y, w, h] };
}

/** The angle (degrees) that makes the line a→b (screen px) horizontal or vertical, whichever is nearer. */
export function levelAngle(ax: number, ay: number, bx: number, by: number): number | null {
  const dx = bx - ax, dy = by - ay;
  if (Math.hypot(dx, dy) < 10) return null;
  let deg = (Math.atan2(dy, dx) * 180) / Math.PI; // -180..180
  // Fold onto the nearest axis: -45..45 from horizontal or vertical.
  deg = ((deg % 90) + 90) % 90;
  if (deg > 45) deg -= 90;
  return -deg;
}

export const ASPECTS: { label: string; ratio: number | "original" | null }[] = [
  { label: "Free", ratio: null },
  { label: "Original", ratio: "original" },
  { label: "1:1", ratio: 1 },
  { label: "4:5", ratio: 4 / 5 },
  { label: "3:4", ratio: 3 / 4 },
  { label: "5:7", ratio: 5 / 7 },
  { label: "2:3", ratio: 2 / 3 },
  { label: "9:16", ratio: 9 / 16 },
];
