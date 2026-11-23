// Colour grading: where the three dots (shadows, midtones, highlights) sit on the wheel, and how
// they follow each other while they are linked in a shape. The picture is made from the three
// dots alone (`tonality_core::grading`); the link only decides how they move here.
import type { ColorGrading } from "$lib/bindings/ColorGrading";
import type { GradeRange } from "$lib/bindings/GradeRange";
import type { Harmony } from "$lib/bindings/Harmony";

export type Range = "shadows" | "midtones" | "highlights";
export const RANGES: Range[] = ["shadows", "midtones", "highlights"];

/** `edit.grading` is absent until first touched (so untouched states keep their hashes). */
export const GRADING_DEFAULT: ColorGrading = {
  shadows: { hue: 0, saturation: 0, luminance: 0 },
  midtones: { hue: 0, saturation: 0, luminance: 0 },
  highlights: { hue: 0, saturation: 0, luminance: 0 },
  blending: 50,
  balance: 0,
  link: null,
};

/**
 * Each shape: the hue of every range, in degrees round from the shadows. `null` is the centre of
 * the wheel (a line's middle). The mirror image of a shape counts the other way round.
 */
const SHAPES: Record<Harmony, Record<Range, number | null>> = {
  analogous: { shadows: 0, midtones: 30, highlights: 60 },
  complementary: { shadows: 0, midtones: null, highlights: 180 },
  triadic: { shadows: 0, midtones: 120, highlights: 240 },
  splitComplementary: { shadows: 0, midtones: 150, highlights: 210 },
};

export const HARMONIES: { id: Harmony; label: string }[] = [
  { id: "analogous", label: "Analogous" },
  { id: "complementary", label: "Complementary" },
  { id: "triadic", label: "Triadic" },
  { id: "splitComplementary", label: "Split-complementary" },
];

const wrap = (hue: number) => ((Math.round(hue) % 360) + 360) % 360;
const clampSat = (s: number) => Math.min(100, Math.max(0, Math.round(s)));
/** Degrees between two hues, the short way round. */
const apart = (a: number, b: number) => Math.abs(((a - b + 540) % 360) - 180);

/** Whether the link holds `range` at the centre of the wheel (the midtones, on a line). */
export function heldAtCentre(g: ColorGrading, range: Range): boolean {
  return !!g.link && SHAPES[g.link][range] === null;
}

/** Where `range` goes in `harmony` built around `anchor` at `hue`. */
function target(harmony: Harmony, anchor: Range, hue: number, range: Range, mirrored: boolean) {
  const shape = SHAPES[harmony];
  const [from, to] = [shape[anchor], shape[range]];
  if (from === null || to === null) return null;
  return wrap(hue + (mirrored ? from - to : to - from));
}

/**
 * Which way round `harmony` sits closest to where the dots are now: `true` for its mirror image.
 * `counted` picks the dots that have a say.
 */
function isMirrored(
  g: ColorGrading,
  harmony: Harmony,
  anchor: Range,
  counted: (r: Range) => boolean,
): boolean {
  const cost = (mirrored: boolean) =>
    RANGES.filter((r) => r !== anchor && counted(r)).reduce((sum, r) => {
      const t = target(harmony, anchor, g[anchor].hue, r, mirrored);
      return t === null ? sum : sum + apart(g[r].hue, t);
    }, 0);
  return cost(true) < cost(false);
}

/** `g` with the other dots placed on `harmony` around `anchor`, all as far out as it is. */
function placed(g: ColorGrading, harmony: Harmony, anchor: Range, mirrored: boolean): ColorGrading {
  const out = { ...g };
  for (const r of RANGES) {
    const t = target(harmony, anchor, g[anchor].hue, r, mirrored);
    out[r] =
      t === null
        ? { ...g[r], saturation: 0 }
        : { ...g[r], hue: t, saturation: g[anchor].saturation };
  }
  return out;
}

/**
 * The dot a shape is built around when the link goes on: `selected` if it has colour, else one
 * that does, so that linking never throws a grade away. Never a dot the shape holds at the centre.
 */
export function anchorFor(g: ColorGrading, harmony: Harmony, selected: Range): Range {
  const able = [selected, ...RANGES].filter((r) => SHAPES[harmony][r] !== null);
  return able.find((r) => g[r].saturation > 0) ?? able[0];
}

/**
 * Link the dots in `harmony` (or switch to it), or unlink them with `null`. Linking moves the
 * other dots onto the shape, the way round that moves them least; switching shape keeps the way
 * round it was; unlinking moves nothing.
 */
export function linked(g: ColorGrading, harmony: Harmony | null, selected: Range): ColorGrading {
  if (!harmony) return { ...g, link: null };
  const anchor = anchorFor(g, harmony, selected);
  // Already in a shape: the new one goes round the same way. Otherwise whichever way is nearer,
  // going by the dots that have colour (one at the centre has no hue to speak of).
  const mirrored = g.link
    ? isMirrored(g, g.link, anchor, () => true)
    : isMirrored(g, harmony, anchor, (r) => g[r].saturation > 0);
  return { ...placed(g, harmony, anchor, mirrored), link: harmony };
}

/**
 * Move `range` to `hue` (degrees) and `saturation` (0–100). Linked, the shape turns and grows
 * with it; a dot held at the centre doesn't move.
 */
export function moved(
  g: ColorGrading,
  range: Range,
  hue: number,
  saturation: number,
): ColorGrading {
  const at = { ...g[range], hue: wrap(hue), saturation: clampSat(saturation) };
  if (!g.link) return { ...g, [range]: at };
  if (heldAtCentre(g, range)) return g;
  const mirrored = isMirrored(g, g.link, range, () => true);
  return placed({ ...g, [range]: at }, g.link, range, mirrored);
}

/** The linked shape's mirror image, around `selected` where it can hold still. */
export function flipped(g: ColorGrading, selected: Range): ColorGrading {
  if (!g.link) return g;
  const anchor = heldAtCentre(g, selected) ? "shadows" : selected;
  return placed(g, g.link, anchor, !isMirrored(g, g.link, anchor, () => true));
}

/** A dot's place on the wheel: the unit circle, hue 0 to the right, anticlockwise, y up. */
export function toPoint(r: GradeRange): { x: number; y: number } {
  const [a, d] = [(r.hue * Math.PI) / 180, r.saturation / 100];
  return { x: d * Math.cos(a), y: d * Math.sin(a) };
}

/** The hue and saturation at a point on the wheel; beyond the rim counts as on it. */
export function fromPoint(x: number, y: number): { hue: number; saturation: number } {
  return {
    hue: wrap((Math.atan2(y, x) * 180) / Math.PI),
    saturation: clampSat(Math.hypot(x, y) * 100),
  };
}
