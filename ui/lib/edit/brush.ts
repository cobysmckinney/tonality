// Live brush painting for the preview. A port of `stamp_segment` in
// crates/tonality-core/src/mask.rs: while a stroke is being drawn the preview paints it here;
// once committed, the Rust rasteriser's layers replace it (brush.test.ts keeps the two equal).
import type { Stroke } from "$lib/bindings/Stroke";

export interface DirtyRect {
  x: number;
  y: number;
  w: number;
  h: number;
}

function smoothstep(e0: number, e1: number, x: number) {
  const t = Math.min(1, Math.max(0, (x - e0) / (e1 - e0)));
  return t * t * (3 - 2 * t);
}

const f32 = Math.fround;

/** Sweep the brush from `a` to `b` (source uv) into an 8-bit `w × h` layer. Returns what changed. */
export function stampSegment(
  layer: Uint8Array,
  w: number,
  h: number,
  a: readonly number[],
  b: readonly number[],
  s: Stroke,
): DirtyRect | null {
  const long = Math.max(w, h);
  const r = f32(Math.max(f32(s.radius * long), 0.5));
  const inner = f32(Math.min(f32(r * f32(1 - Math.min(1, Math.max(0, s.feather)))), r - 1));
  const ax = f32(a[0] * w), ay = f32(a[1] * h);
  const bx = f32(b[0] * w), by = f32(b[1] * h);
  const x0 = Math.max(0, Math.floor(Math.min(ax, bx) - r - 1));
  const y0 = Math.max(0, Math.floor(Math.min(ay, by) - r - 1));
  const x1 = Math.min(w, Math.ceil(Math.max(ax, bx) + r + 1));
  const y1 = Math.min(h, Math.ceil(Math.max(ay, by) + r + 1));
  if (x1 <= x0 || y1 <= y0) return null;
  const dx = f32(bx - ax), dy = f32(by - ay);
  const len2 = f32(dx * dx + dy * dy);
  const strength = Math.min(1, Math.max(0, s.strength));
  for (let y = y0; y < y1; y++) {
    for (let x = x0; x < x1; x++) {
      const px = f32(x + 0.5 - ax), py = f32(y + 0.5 - ay);
      const t = len2 > 0 ? Math.min(1, Math.max(0, f32(f32(px * dx + py * dy) / len2))) : 0;
      const ex = f32(px - t * dx), ey = f32(py - t * dy);
      const d = f32(Math.sqrt(ex * ex + ey * ey));
      if (d >= r) continue;
      const v = strength * (1 - smoothstep(inner, r, d));
      const q = Math.round(v * 255);
      const i = y * w + x;
      layer[i] = s.erase ? Math.min(layer[i], 255 - q) : Math.max(layer[i], q);
    }
  }
  return { x: x0, y: y0, w: x1 - x0, h: y1 - y0 };
}

/** Paint a whole stroke. */
export function paint(layer: Uint8Array, w: number, h: number, s: Stroke) {
  const n = s.points.length;
  if (n === 1) stampSegment(layer, w, h, s.points[0], s.points[0], s);
  for (let i = 1; i < n; i++) stampSegment(layer, w, h, s.points[i - 1], s.points[i], s);
}
