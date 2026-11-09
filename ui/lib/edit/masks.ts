// Masks: creating them, naming them, and mapping between the screen and source uv.
import type { EditState } from "$lib/bindings/EditState";
import type { Mask } from "$lib/bindings/Mask";
import type { MaskShape } from "$lib/bindings/MaskShape";
import type { Framing } from "./renderer";

export type MaskKind = MaskShape["kind"];
/** Mirrors `MAX_MASKS` in tonality-render. */
export const MAX_MASKS = 16;

export const KIND_LABEL: Record<MaskKind, string> = { linear: "Linear", radial: "Radial", brush: "Brush" };

export type Vec2 = [number, number];

/** Frame position (0..1 across the framed output) → source uv. */
export function toUv(fr: Framing, f: Vec2): Vec2 {
  const m = fr.m;
  return [m[0] * f[0] + m[1] * f[1] + m[2], m[3] * f[0] + m[4] * f[1] + m[5]];
}

/** Source uv → frame position. */
export function toFrame(fr: Framing, uv: Vec2): Vec2 {
  const [a, b, c, d, e, f] = fr.m;
  const det = a * e - b * d || 1e-12;
  const x = uv[0] - c, y = uv[1] - f;
  return [(e * x - b * y) / det, (a * y - d * x) / det];
}

/** Source size / its long edge: uv × this = long-edge units (where circles are round). */
export function norm(fullW: number, fullH: number): Vec2 {
  const l = Math.max(fullW, fullH, 1);
  return [fullW / l, fullH / l];
}

export function masksOf(edit: EditState): Mask[] {
  return edit.masks ?? [];
}

/** A new mask's name: "Radial 2" etc. */
export function nextName(masks: Mask[], kind: MaskKind): string {
  const label = KIND_LABEL[kind];
  let n = 0;
  for (const m of masks) {
    const k = m.name.match(new RegExp(`^${label} (\\d+)$`));
    if (k) n = Math.max(n, Number(k[1]));
  }
  return `${label} ${n + 1}`;
}

/**
 * A new mask of `kind`, placed in the middle of the current frame and lined up with the screen:
 * a radial in the centre, a linear fading down from the top (a sky).
 */
export function newMask(kind: MaskKind, masks: Mask[], fr: Framing, fullW: number, fullH: number): Mask {
  const nm = norm(fullW, fullH);
  const q = (f: Vec2): Vec2 => {
    const uv = toUv(fr, f);
    return [uv[0] * nm[0], uv[1] * nm[1]];
  };
  const r4 = (v: number) => Math.round(v * 1e4) / 1e4;
  const uv = (f: Vec2) => toUv(fr, f).map(r4) as Vec2;
  let shape: MaskShape;
  if (kind === "linear") {
    shape = { kind, from: uv([0.5, 0.05]), to: uv([0.5, 0.45]) };
  } else if (kind === "radial") {
    // The frame's horizontal, in long-edge units, sets the ellipse's angle and scale.
    const c = q([0.5, 0.5]), e = q([0.6, 0.5]), v = q([0.5, 0.6]);
    const angle = (Math.atan2(e[1] - c[1], e[0] - c[0]) * 180) / Math.PI;
    const rx = Math.hypot(e[0] - c[0], e[1] - c[1]) * 2.5;
    const ry = Math.hypot(v[0] - c[0], v[1] - c[1]) * 2.5;
    shape = { kind, center: uv([0.5, 0.5]), radius: [r4(rx), r4(ry)], angle: r4(angle), feather: 0.5 };
  } else {
    shape = { kind, strokes: [] };
  }
  return {
    name: nextName(masks, kind),
    enabled: true,
    shape,
    invert: false,
    amount: 100,
    range: { low: 0, high: 100, smoothness: 20 },
    adjust: { exposure: 0, contrast: 0, highlights: 0, shadows: 0, whites: 0, blacks: 0, temperature: 0, tint: 0, saturation: 0 },
  };
}

/** Index of mask `i`'s raster among the brush layers (every brush mask has one). */
export function brushLayer(masks: Mask[], i: number): number {
  let n = 0;
  for (let k = 0; k < i; k++) if (masks[k].shape.kind === "brush") n++;
  return n;
}

/** Index of mask `i` in the shader (hidden masks are skipped), or null if it isn't drawn. */
export function shaderIndex(masks: Mask[], i: number): number | null {
  if (!masks[i]?.enabled) return null;
  let n = 0;
  for (let k = 0; k < i; k++) if (masks[k].enabled) n++;
  return n < MAX_MASKS ? n : null;
}

/** Identifies the brush rasters a state needs (so they're only refetched when strokes change). */
export function brushKey(edit: EditState): string {
  const strokes = masksOf(edit).flatMap((m) => (m.shape.kind === "brush" ? [m.shape.strokes] : []));
  return strokes.length ? JSON.stringify(strokes) : "";
}

/** Mirrors `MASK_EDGE` / `mask_size` in tonality-core: the brush raster size. */
export const MASK_EDGE = 1024;
export function maskSize(fullW: number, fullH: number): [number, number] {
  const s = Math.fround(MASK_EDGE / Math.max(fullW, fullH, 1));
  return [Math.max(1, Math.round(Math.fround(fullW * s))), Math.max(1, Math.round(Math.fround(fullH * s)))];
}

/** Brush rasters, as sent by `mask_layers`. */
export interface BrushLayers {
  width: number;
  height: number;
  layers: number;
  data: Uint8Array;
}

/** Brush settings (not part of the edit: they shape new strokes). */
export interface BrushTool {
  /** Radius, long-edge units. */
  size: number;
  /** 0..1 of the radius. */
  feather: number;
  /** 0..1 */
  strength: number;
  erase: boolean;
}
