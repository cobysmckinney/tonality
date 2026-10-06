/**
 * Masks in the interface: making them, naming them, and the arithmetic that
 * places them on screen. Shapes are kept on the photo file (see Shape in
 * adjustments.ts), so they stay on the same part of the picture whatever the
 * crop, turn or flip; this maps between the file and the screen. It mirrors
 * `frame_to_source` in src-tauri/src/geometry.rs.
 */
import {
  Adjustments,
  formatValue,
  LABELS,
  LocalAdjustments,
  LOCAL_KEYS,
  Mask,
  MaskMode,
  noLocalAdjustments,
  Point,
  Shape,
} from "./adjustments";
import { Size, turnedSize } from "./crop";

/** An affine map of the plane, as two rows: x' = a*x + b*y + c, y' = d*x + e*y + f. */
export type Affine = [[number, number, number], [number, number, number]];

export const apply = ([[a, b, c], [d, e, f]]: Affine, [x, y]: Point): Point => [a * x + b * y + c, d * x + e * y + f];

/** The map that does `first`, then `next`. */
export function then(first: Affine, next: Affine): Affine {
  const [[a, b, c], [d, e, f]] = first;
  const [[g, h, i], [j, k, l]] = next;
  return [
    [g * a + h * d, g * b + h * e, g * c + h * f + i],
    [j * a + k * d, j * b + k * e, j * c + k * f + l],
  ];
}

export function invert([[a, b, c], [d, e, f]]: Affine): Affine {
  const det = a * e - b * d;
  return [
    [e / det, -b / det, (b * f - c * e) / det],
    [-d / det, a / det, (c * d - a * f) / det],
  ];
}

const scale = (sx: number, sy: number): Affine => [
  [sx, 0, 0],
  [0, sy, 0],
];
const translate = (dx: number, dy: number): Affine => [
  [1, 0, dx],
  [0, 1, dy],
];
const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

/** Maps a position in the cropped picture (0..1 across and down) to a position on the photo file. */
export function frameToSource(photo: Size, a: Adjustments): Affine {
  const { width: w, height: h } = turnedSize(photo, a.rotation);
  const angle = (clamp(a.straighten, -45, 45) * Math.PI) / 180;
  const [sin, cos] = [Math.sin(angle), Math.cos(angle)];
  const part = (value: number) => clamp(value, 0, 1);
  const size = [Math.max(part(a.crop.width), 0.01) * w, Math.max(part(a.crop.height), 0.01) * h];
  const center = [part(a.crop.x) * w, part(a.crop.y) * h];

  const offset = then(translate(-0.5, -0.5), scale(size[0], size[1]));
  const tilt: Affine = [
    [cos, sin, 0],
    [-sin, cos, 0],
  ];
  const onTurned = then(then(offset, tilt), translate(center[0], center[1]));
  const flip: Affine = [
    [a.flipHorizontal ? -1 : 1, 0, a.flipHorizontal ? 1 : 0],
    [0, a.flipVertical ? -1 : 1, a.flipVertical ? 1 : 0],
  ];
  const unturn: Affine = (
    [
      [
        [1, 0, 0],
        [0, 1, 0],
      ],
      [
        [0, 1, 0],
        [-1, 0, 1],
      ],
      [
        [-1, 0, 1],
        [0, -1, 1],
      ],
      [
        [0, -1, 1],
        [1, 0, 0],
      ],
    ] as Affine[]
  )[((a.rotation % 4) + 4) % 4];
  return then(then(then(onTurned, scale(1 / w, 1 / h)), flip), unturn);
}

/** What the screen shows: which part of the cropped picture, at what size in CSS pixels. */
export interface ScreenView {
  photo: Size;
  adjustments: Adjustments;
  region: { x: number; y: number; width: number; height: number };
  shown: Size;
}

/** Between the photo file and the screen. */
export interface ScreenMap {
  /** A position on the file to a position on screen. */
  toScreen: (point: Point) => Point;
  /** A position on screen back to the file. */
  toPhoto: (point: Point) => Point;
  /** Screen pixels per share of the photo's longer side. */
  scale: number;
  /** A direction in the file's pixels, as a direction on screen. */
  direction: (vector: Point) => Point;
  /** A position on the file in its own pixels, and back. */
  toPixels: (point: Point) => Point;
  fromPixels: (pixels: Point) => Point;
  /** The photo's longer side, in pixels. */
  long: number;
}

export function screenMap({ photo, adjustments, region, shown }: ScreenView): ScreenMap {
  const frameToScreen: Affine = [
    [shown.width / region.width, 0, (-region.x * shown.width) / region.width],
    [0, shown.height / region.height, (-region.y * shown.height) / region.height],
  ];
  const fileToScreen = then(invert(frameToSource(photo, adjustments)), frameToScreen);
  const screenToFile = invert(fileToScreen);
  const pixelsToScreen = then(scale(1 / photo.width, 1 / photo.height), fileToScreen);
  const long = Math.max(photo.width, photo.height);
  const [[a, b], [d, e]] = pixelsToScreen;
  return {
    toScreen: (point) => apply(fileToScreen, point),
    toPhoto: (point) => apply(screenToFile, point),
    scale: Math.hypot(a, d) * long,
    direction: ([x, y]) => [a * x + b * y, d * x + e * y],
    toPixels: ([x, y]) => [x * photo.width, y * photo.height],
    fromPixels: ([x, y]) => [x / photo.width, y / photo.height],
    long,
  };
}

/** What each kind of part is called. */
export const SHAPE_NAMES: Record<Shape["kind"], string> = {
  brush: "Brush",
  linear: "Linear gradient",
  radial: "Radial gradient",
  luminance: "Brightness range",
  subject: "Subject",
  sky: "Sky",
  object: "Object",
};

/** What a new mask can start from: a kind of part, or the background (the subject, inverted). */
export type MaskStart = Shape["kind"] | "background";

/** Whether a part is found by a model (from the photo, or inside a circle) rather than drawn. */
export const isFound = (shape: Shape) => shape.kind === "subject" || shape.kind === "sky" || shape.kind === "object";

/** The found parts among these masks' parts. */
export const foundShapes = (masks: Mask[]) => masks.flatMap((mask) => mask.parts.map((part) => part.shape)).filter(isFound);

export const MODE_NAMES: Record<MaskMode, string> = { add: "Add", subtract: "Subtract", intersect: "Intersect" };

/** The most masks, and parts of each kind, the editor draws; mirrors masks.rs. */
export const MAX_MASKS = 8;
export const MAX_PARTS = 32;
export const MAX_BRUSHES = 8;
/** Different found parts: the same subject twice counts once. */
export const MAX_FOUND = 8;

/** Keeps stored positions short: a hundred-thousandth of the photo is finer than a pixel. */
export const tidy = (value: number) => Math.round(value * 1e5) / 1e5;

/**
 * A new part of a kind, placed in the middle of the cropped picture and
 * sized to it: a gradient falling from a third of the way down to about
 * two thirds, a circle a third of the picture across.
 */
export function newShape(kind: Shape["kind"], photo: Size, adjustments: Adjustments): Shape {
  const toFile = frameToSource(photo, adjustments);
  const at = (x: number, y: number): Point => apply(toFile, [x, y]).map(tidy) as Point;
  const { width, height } = turnedSize(photo, adjustments.rotation);
  const long = Math.max(photo.width, photo.height);
  switch (kind) {
    case "brush":
      return { kind, strokes: [] };
    case "linear":
      return { kind, from: at(0.5, 0.3), to: at(0.5, 0.62) };
    case "radial": {
      const across = Math.min(adjustments.crop.width * width, adjustments.crop.height * height);
      const r = tidy((across * 0.33) / long);
      return { kind, center: at(0.5, 0.5), radius: [r, r], angle: 0, feather: 0.5 };
    }
    case "luminance":
      return { kind, low: 0, high: 0.35, smoothness: 0.15 };
    case "subject":
    case "sky":
      return { kind };
    case "object":
      // Its outline comes from the circle drawn on the photo (`circleShape`).
      return { kind, points: [] };
  }
}

/** Keeps a circle short: a point every so often along it, tidied, at most `limit` of them. */
export function circleShape(points: Point[], limit = 160): Shape {
  const step = Math.max(1, Math.ceil(points.length / limit));
  return { kind: "object", points: points.filter((_, i) => i % step === 0).map((p) => p.map(tidy) as Point) };
}

/** The kind of part a new mask starts with. */
export const startKind = (start: MaskStart): Shape["kind"] => (start === "background" ? "subject" : start);

/**
 * A new mask holding one part, with an id and a name no other mask has:
 * "Mask 1", or for the subject and background, "Subject" and "Background".
 */
export function newMask(masks: Mask[], start: MaskStart, photo: Size, adjustments: Adjustments, shape?: Shape): Mask {
  const id = Math.max(0, ...masks.map((mask) => mask.id)) + 1;
  const names = new Set(masks.map((mask) => mask.name));
  const base = start === "background" ? "Background" : start === "subject" || start === "sky" || start === "object" ? SHAPE_NAMES[start] : null;
  let name = base ?? "Mask 1";
  for (let n = 2; names.has(name); n++) name = base ? `${base} ${n}` : `Mask ${n}`;
  return {
    id,
    name,
    visible: true,
    invert: start === "background",
    // A brightness range on its own covers the range; it intersects only once something comes before it.
    parts: [{ mode: "add", shape: shape ?? newShape(startKind(start), photo, adjustments) }],
    adjustments: noLocalAdjustments(),
  };
}

/** The mode a part of this kind starts with when added to a mask that already has parts. */
export const defaultMode = (kind: Shape["kind"]): MaskMode => (kind === "luminance" ? "intersect" : "add");

/** The brush size slider (1..100) as a radius, in shares of the photo's longer side. Finer at the small end. */
export const brushRadius = (size: number) => 0.25 * (clamp(size, 1, 100) / 100) ** 2;

/** What the history calls a change to one of a mask's sliders. */
export function describeLocal(mask: Mask, before: LocalAdjustments): string {
  const changed = LOCAL_KEYS.filter((key) => mask.adjustments[key] !== before[key]);
  if (changed.length !== 1) return `${mask.name}: adjustments`;
  const key = changed[0];
  return `${mask.name}: ${LABELS[key]} ${formatValue(mask.adjustments[key], key === "exposure" ? 0.01 : 1)}`;
}

/** How many brush parts there are across all masks. */
export const brushCount = (masks: Mask[]) =>
  masks.reduce((n, mask) => n + mask.parts.filter((part) => part.shape.kind === "brush").length, 0);

export const partCount = (masks: Mask[]) => masks.reduce((n, mask) => n + mask.parts.length, 0);

/** How many different found parts there are across all masks. */
export const foundCount = (masks: Mask[]) => new Set(foundShapes(masks).map((shape) => JSON.stringify(shape))).size;

/** Whether another part of this kind still fits in what the editor draws. */
export function canAdd(masks: Mask[], kind: Shape["kind"]): boolean {
  if (partCount(masks) >= MAX_PARTS) return false;
  if (kind === "brush") return brushCount(masks) < MAX_BRUSHES;
  // The subject or sky may already be among them, and so not count again.
  if (kind === "object" || ((kind === "subject" || kind === "sky") && !foundShapes(masks).some((s) => s.kind === kind))) {
    return foundCount(masks) < MAX_FOUND;
  }
  return true;
}
