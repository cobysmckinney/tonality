/** A tone curve as control points from [0, 0] (black) to [1, 1] (white). */
export type CurvePoints = [number, number][];
export type CurveChannel = "master" | "red" | "green" | "blue";

export interface Band {
  hue: number;
  saturation: number;
  luminance: number;
}

/**
 * The part of the photo that is kept: centre and size as fractions of the
 * photo after its quarter-turns and flips, tilted by the straighten angle.
 */
export interface Crop {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** A position on the photo file: 0..1 across and down, before any turn, flip or crop. */
export type Point = [number, number];

/** One brush stroke. Mirrors `Stroke` in edit.rs. */
export interface Stroke {
  points: Point[];
  /** The brush's radius, as a share of the photo's longer side. */
  radius: number;
  /** 0..1: how much of the radius fades out. */
  feather: number;
  /** 0..1: how much the stroke covers at its centre. */
  strength: number;
  /** Takes coverage away instead of adding it. */
  erase: boolean;
}

/**
 * What a part of a mask covers. Sizes are shares of the photo's longer side;
 * angles are degrees clockwise on the file as stored. Mirrors `Shape` in edit.rs.
 */
export type Shape =
  | { kind: "brush"; strokes: Stroke[] }
  /** Full effect at `from`, none at `to`. */
  | { kind: "linear"; from: Point; to: Point }
  /** An ellipse fading out over its outer `feather` (0..1) share. */
  | { kind: "radial"; center: Point; radius: [number, number]; angle: number; feather: number }
  /** Parts of the photo within a range of brightness, 0 black to 1 white. */
  | { kind: "luminance"; low: number; high: number; smoothness: number };

/** How a part combines with the parts before it. */
export type MaskMode = "add" | "subtract" | "intersect";

export interface MaskPart {
  mode: MaskMode;
  shape: Shape;
}

/** A local adjustment: an area of the photo and the sliders that apply there. Mirrors `Mask` in edit.rs. */
export interface Mask {
  /** Unique within a recipe. */
  id: number;
  name: string;
  /** A hidden mask stays in the recipe but has no effect. */
  visible: boolean;
  /** Applies everywhere the parts do not cover instead. */
  invert: boolean;
  parts: MaskPart[];
  adjustments: LocalAdjustments;
}

/**
 * Every slider in the editor; zero everywhere means "as shot". Exposure is in
 * stops (-5..5), everything else runs -100..100 or 0..100. Mirrors
 * `Adjustments` in the backend's edit.rs.
 */
export interface Adjustments {
  exposure: number;
  contrast: number;
  highlights: number;
  shadows: number;
  whites: number;
  blacks: number;
  temperature: number;
  tint: number;
  vibrance: number;
  saturation: number;
  curves: Record<CurveChannel, CurvePoints>;
  mixer: Band[];
  clarity: number;
  dehaze: number;
  sharpening: number;
  noiseReduction: number;
  vignette: number;
  grain: number;
  crop: Crop;
  /** Degrees, -45..45; positive turns the picture clockwise. */
  straighten: number;
  /** Quarter-turns clockwise, 0..3. */
  rotation: number;
  flipHorizontal: boolean;
  flipVertical: boolean;
  /** Local adjustments, laid over the rest in order. */
  masks: Mask[];
}

/** The adjustments that reshape the picture rather than recolour it. */
export const GEOMETRY = ["crop", "straighten", "rotation", "flipHorizontal", "flipVertical"] as const;

/** The adjustments that are plain numbers, i.e. everything a simple slider edits. */
export type SliderKey = {
  [K in keyof Adjustments]: Adjustments[K] extends number ? K : never;
}[keyof Adjustments];

/** The sliders a mask can carry. Each adds to the photo's own setting wherever the mask covers. */
export const LOCAL_KEYS = [
  "exposure",
  "contrast",
  "highlights",
  "shadows",
  "whites",
  "blacks",
  "temperature",
  "tint",
  "vibrance",
  "saturation",
  "clarity",
  "dehaze",
  "sharpening",
  "noiseReduction",
] as const satisfies readonly SliderKey[];
export type LocalKey = (typeof LOCAL_KEYS)[number];
export type LocalAdjustments = Record<LocalKey, number>;

export const noLocalAdjustments = (): LocalAdjustments =>
  Object.fromEntries(LOCAL_KEYS.map((key) => [key, 0])) as LocalAdjustments;

const straight = (): CurvePoints => [
  [0, 0],
  [1, 1],
];

export const MIXER_BANDS = [
  { name: "Red", color: "#e5484d" },
  { name: "Orange", color: "#f08a3c" },
  { name: "Yellow", color: "#e8c53a" },
  { name: "Green", color: "#4fb866" },
  { name: "Aqua", color: "#3fbfc4" },
  { name: "Blue", color: "#4a7fe8" },
  { name: "Purple", color: "#8b62e0" },
  { name: "Magenta", color: "#d756b8" },
] as const;

export function defaultAdjustments(): Adjustments {
  return {
    exposure: 0,
    contrast: 0,
    highlights: 0,
    shadows: 0,
    whites: 0,
    blacks: 0,
    temperature: 0,
    tint: 0,
    vibrance: 0,
    saturation: 0,
    curves: { master: straight(), red: straight(), green: straight(), blue: straight() },
    mixer: MIXER_BANDS.map(() => ({ hue: 0, saturation: 0, luminance: 0 })),
    clarity: 0,
    dehaze: 0,
    sharpening: 0,
    noiseReduction: 0,
    vignette: 0,
    grain: 0,
    crop: { x: 0.5, y: 0.5, width: 1, height: 1 },
    straighten: 0,
    rotation: 0,
    flipHorizontal: false,
    flipVertical: false,
    masks: [],
  };
}

export const DEFAULTS: Readonly<Adjustments> = defaultAdjustments();

/** The name each slider goes by, in the panel and in the history. */
export const LABELS: Record<SliderKey, string> = {
  exposure: "Exposure",
  contrast: "Contrast",
  highlights: "Highlights",
  shadows: "Shadows",
  whites: "Whites",
  blacks: "Blacks",
  temperature: "Temperature",
  tint: "Tint",
  vibrance: "Vibrance",
  saturation: "Saturation",
  clarity: "Clarity",
  dehaze: "Dehaze",
  sharpening: "Sharpening",
  noiseReduction: "Noise reduction",
  vignette: "Vignette",
  grain: "Grain",
  straighten: "Straighten",
  rotation: "Rotate",
};

/** How far a slider runs, and in what steps: -100..100 in whole numbers unless it says otherwise here. */
const RANGES: Partial<Record<SliderKey, { min?: number; max?: number; step?: number }>> = {
  exposure: { min: -5, max: 5, step: 0.01 },
  sharpening: { min: 0 },
  noiseReduction: { min: 0 },
  grain: { min: 0 },
};
export const rangeOf = (key: SliderKey) => ({ min: -100, max: 100, step: 1, ...RANGES[key] });

/** The groups of tools in the adjust panel, in order. */
export const SECTIONS: { title: string; keys: (keyof Adjustments)[] }[] = [
  { title: "Light", keys: ["exposure", "contrast", "highlights", "shadows", "whites", "blacks"] },
  { title: "Color", keys: ["temperature", "tint", "vibrance", "saturation"] },
  { title: "Curve", keys: ["curves"] },
  { title: "Color mixer", keys: ["mixer"] },
  { title: "Detail", keys: ["sharpening", "noiseReduction", "clarity", "dehaze"] },
  { title: "Effects", keys: ["vignette", "grain"] },
  { title: "Crop", keys: [...GEOMETRY] },
];

/** A slider value as shown: signed, with as many decimals as the slider's step has. */
export function formatValue(value: number, step = 1): string {
  const text = Math.abs(value).toFixed(step < 0.1 ? 2 : step < 1 ? 1 : 0);
  if (Number(text) === 0) return text;
  return `${value > 0 ? "+" : "−"}${text}`;
}

/**
 * Whether two values (a slider, a curve, a whole recipe) are the same,
 * whatever order their fields happen to be written in.
 */
export function same(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  const [x, y] = [a as Record<string, unknown>, b as Record<string, unknown>];
  const keys = Object.keys(x);
  return keys.length === Object.keys(y).length && keys.every((key) => key in y && same(x[key], y[key]));
}

/** Names the difference between two recipes for the history: "Exposure +0.50". */
export function describeChange(before: Adjustments, after: Adjustments): string {
  const changed = (Object.keys(after) as (keyof Adjustments)[]).filter((key) => !same(before[key], after[key]));
  if (changed.length === 0) return "No change";
  if (isAsShot(after)) return "Reset all";

  if (changed.every((key) => (GEOMETRY as readonly string[]).includes(key))) {
    if (GEOMETRY.every((key) => same(after[key], DEFAULTS[key]))) return "Reset crop";
    if (changed.includes("rotation")) return "Rotate";
    if (changed.includes("flipHorizontal") || changed.includes("flipVertical")) return "Flip";
    if (changed.includes("straighten")) return `Straighten ${formatValue(after.straighten, 0.1)}°`;
    return "Crop";
  }

  if (changed.length === 1) {
    const key = changed[0];
    // The masks panel names its own steps; this is only a fallback.
    if (key === "masks") return "Masks";
    if (key === "curves") {
      if (same(after.curves, DEFAULTS.curves)) return "Reset curve";
      const channels = (Object.keys(after.curves) as CurveChannel[]).filter((c) => !same(before.curves[c], after.curves[c]));
      const only = channels.length === 1 && channels[0] !== "master" ? channels[0] : null;
      return only ? `${only[0].toUpperCase()}${only.slice(1)} curve` : "Tone curve";
    }
    if (key === "mixer") {
      if (same(after.mixer, DEFAULTS.mixer)) return "Reset color mixer";
      const bands = after.mixer.map((band, i) => (same(band, before.mixer[i]) ? null : i)).filter((i) => i !== null);
      return bands.length === 1 ? `Color mixer: ${MIXER_BANDS[bands[0]].name.toLowerCase()}s` : "Color mixer";
    }
    const slider = key as SliderKey;
    return `${LABELS[slider]} ${formatValue(after[slider], slider === "exposure" ? 0.01 : 1)}`;
  }

  // Several at once is a section reset, a paste, or the like.
  const section = SECTIONS.find((s) => changed.every((key) => s.keys.includes(key)));
  if (section && changed.every((key) => same(after[key], DEFAULTS[key]))) return `Reset ${section.title.toLowerCase()}`;
  return `${changed.length} adjustments`;
}

export const sameAdjustments = (a: Adjustments, b: Adjustments) => same(a, b);
export const isAsShot = (a: Adjustments) => sameAdjustments(a, DEFAULTS);

/** How close together two points of a curve may be. */
export const POINT_GAP = 0.02;

export const isStraight = (points: CurvePoints) =>
  points.length === 2 && points[0][0] === 0 && points[0][1] === 0 && points[1][0] === 1 && points[1][1] === 1;

/**
 * Evaluates a curve at `x`. The same monotone cubic spline the backend uses
 * to render (see `curve_table` in edit.rs), here only to draw the line.
 */
export function curveSampler(input: CurvePoints): (x: number) => number {
  const points = [...input].sort((a, b) => a[0] - b[0]).filter((p, i, all) => i === 0 || p[0] - all[i - 1][0] >= 1e-4);
  const n = points.length;
  if (n === 0) return (x) => x;
  if (n === 1) return () => points[0][1];

  const slopes = points.slice(1).map((p, i) => (p[1] - points[i][1]) / (p[0] - points[i][0]));
  const tangents = points.map((_, i) => {
    if (i === 0) return slopes[0];
    if (i === n - 1) return slopes[n - 2];
    return slopes[i - 1] * slopes[i] <= 0 ? 0 : (slopes[i - 1] + slopes[i]) / 2;
  });
  for (let i = 0; i < n - 1; i++) {
    if (slopes[i] === 0) {
      tangents[i] = tangents[i + 1] = 0;
      continue;
    }
    const a = tangents[i] / slopes[i];
    const b = tangents[i + 1] / slopes[i];
    const length = a * a + b * b;
    if (length > 9) {
      const shrink = 3 / Math.sqrt(length);
      tangents[i] = shrink * a * slopes[i];
      tangents[i + 1] = shrink * b * slopes[i];
    }
  }

  return (x) => {
    if (x <= points[0][0]) return points[0][1];
    if (x >= points[n - 1][0]) return points[n - 1][1];
    let k = 0;
    while (points[k + 1][0] <= x) k++;
    const width = points[k + 1][0] - points[k][0];
    const t = (x - points[k][0]) / width;
    const t2 = t * t;
    const t3 = t2 * t;
    const y =
      (2 * t3 - 3 * t2 + 1) * points[k][1] +
      (t3 - 2 * t2 + t) * width * tangents[k] +
      (-2 * t3 + 3 * t2) * points[k + 1][1] +
      (t3 - t2) * width * tangents[k + 1];
    return Math.min(1, Math.max(0, y));
  };
}
