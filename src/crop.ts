/**
 * Crop arithmetic for the crop tool. Mirrors src-tauri/src/geometry.rs, which
 * is what actually draws the picture: the two must agree on what a crop means.
 *
 * Everything here is measured on the "turned" photo (after quarter-turns and
 * flips), in its pixels. The crop is an upright rectangle in a frame tilted
 * by the straighten angle; "frame" directions are what you see on screen in
 * the crop tool, "photo" directions are along the turned photo's own edges.
 */
import { Adjustments, Crop, describeChange, GEOMETRY, same } from "./adjustments";
import type { Region } from "./api";

export interface Size {
  width: number;
  height: number;
}

/** A crop in pixels: its centre on the turned photo, and its size. */
export interface Rect {
  x: number;
  y: number;
  width: number;
  height: number;
}

/** Which part of the frame a drag has hold of: -1, 0 or 1 on each axis; 0, 0 moves the whole crop. */
export interface Handle {
  x: -1 | 0 | 1;
  y: -1 | 0 | 1;
}

const radians = (degrees: number) => (degrees * Math.PI) / 180;

export const turnedSize = (photo: Size, rotation: number): Size =>
  rotation % 2 === 1 ? { width: photo.height, height: photo.width } : photo;

/** The size of the picture being drawn: the crop, or with `uncropped` the box holding the whole tilted photo. */
export function frameSize(photo: Size, a: Adjustments, uncropped: boolean): Size {
  const turned = turnedSize(photo, a.rotation);
  if (!uncropped) return { width: a.crop.width * turned.width, height: a.crop.height * turned.height };
  const sin = Math.abs(Math.sin(radians(a.straighten)));
  const cos = Math.abs(Math.cos(radians(a.straighten)));
  return { width: turned.width * cos + turned.height * sin, height: turned.width * sin + turned.height * cos };
}

export const toRect = (crop: Crop, turned: Size): Rect => ({
  x: crop.x * turned.width,
  y: crop.y * turned.height,
  width: crop.width * turned.width,
  height: crop.height * turned.height,
});

export const toCrop = (rect: Rect, turned: Size): Crop => ({
  x: rect.x / turned.width,
  y: rect.y / turned.height,
  width: rect.width / turned.width,
  height: rect.height / turned.height,
});

/** A direction on screen (in the frame) as a direction along the photo. */
export function toPhoto([dx, dy]: [number, number], angle: number): [number, number] {
  const sin = Math.sin(radians(angle));
  const cos = Math.cos(radians(angle));
  return [dx * cos + dy * sin, -dx * sin + dy * cos];
}

/** The reverse: a direction along the photo as a direction on screen. */
export function toFrame([qx, qy]: [number, number], angle: number): [number, number] {
  const sin = Math.sin(radians(angle));
  const cos = Math.cos(radians(angle));
  return [qx * cos - qy * sin, qx * sin + qy * cos];
}

/** How far a tilted rectangle reaches from its centre along the photo's own axes. */
function reach(rect: Size, angle: number): [number, number] {
  const sin = Math.abs(Math.sin(radians(angle)));
  const cos = Math.abs(Math.cos(radians(angle)));
  return [(rect.width * cos + rect.height * sin) / 2, (rect.width * sin + rect.height * cos) / 2];
}

/** Whether the tilted crop lies wholly on the photo. */
export function fits(rect: Rect, angle: number, turned: Size): boolean {
  const [rx, ry] = reach(rect, angle);
  const slack = 0.01;
  return (
    rect.x - rx >= -slack && rect.x + rx <= turned.width + slack && rect.y - ry >= -slack && rect.y + ry <= turned.height + slack
  );
}

/** The biggest crop of a given shape, centred on the photo. */
export function largest(aspect: number, angle: number, turned: Size): Rect {
  const sin = Math.abs(Math.sin(radians(angle)));
  const cos = Math.abs(Math.cos(radians(angle)));
  const height = Math.min(turned.width / (aspect * cos + sin), turned.height / (aspect * sin + cos));
  return { x: turned.width / 2, y: turned.height / 2, width: height * aspect, height };
}

/** Scales a crop down about its centre until it lies on the photo. */
export function shrinkToFit(rect: Rect, angle: number, turned: Size): Rect {
  const x = Math.min(Math.max(rect.x, 0), turned.width);
  const y = Math.min(Math.max(rect.y, 0), turned.height);
  const [rx, ry] = reach(rect, angle);
  const scale = Math.min(1, Math.min(x, turned.width - x) / rx, Math.min(y, turned.height - y) / ry);
  return { x, y, width: rect.width * scale, height: rect.height * scale };
}

/** The biggest crop of a given shape around the current crop's centre. */
export function withAspect(rect: Rect, aspect: number, angle: number, turned: Size): Rect {
  const generous = turned.width + turned.height;
  return shrinkToFit({ ...rect, width: generous * aspect, height: generous }, angle, turned);
}

/**
 * Whether a crop stands tall. A crop within a pixel of square has no way up of
 * its own (rounding in the stored crop would decide it), so it follows the photo.
 */
export function isTall(rect: Rect, turned: Size): boolean {
  if (Math.abs(rect.width - rect.height) < 1) return turned.height > turned.width;
  return rect.height > rect.width;
}

/** The width over height a shape gives the crop, kept the way up the crop already is. `ratio` may be either way up. */
export function shapeAspect(ratio: number, rect: Rect, turned: Size): number {
  const long = Math.max(ratio, 1 / ratio);
  return isTall(rect, turned) ? 1 / long : long;
}

/**
 * The chosen shape, if the crop still has it, the way up the crop is now;
 * otherwise null, so the crop is free. The shape isn't part of the history,
 * so after undo, redo or a jump in History the crop may no longer match it,
 * and then it shouldn't hold. The small slack absorbs rounding in the stored crop.
 */
export function heldAspect(rect: Rect, aspect: number | null): number | null {
  if (aspect === null || rect.height <= 0) return null;
  const close = (a: number, b: number) => Math.abs(a / b - 1) < 0.005;
  const ratio = rect.width / rect.height;
  if (close(ratio, aspect)) return aspect;
  if (close(ratio, 1 / aspect)) return 1 / aspect;
  return null;
}

/** Whether a crop is simply "as much as fits": then straightening may grow it again, not only shrink it. */
function isLargest(rect: Rect, angle: number, turned: Size): boolean {
  const full = largest(rect.width / rect.height, angle, turned);
  return (["x", "y", "width", "height"] as const).every((key) => Math.abs(rect[key] - full[key]) < 1);
}

/** The crop to use after the straighten angle changes from `from` to `to`. */
export function straightened(rect: Rect, from: number, to: number, turned: Size): Rect {
  if (isLargest(rect, from, turned)) return largest(rect.width / rect.height, to, turned);
  return shrinkToFit(rect, to, turned);
}

/**
 * The crop after dragging `handle` by `delta` (screen direction, photo
 * pixels) from `start`. The drag stops where the crop would leave the photo.
 * With `aspect` set the shape is kept.
 */
export function dragged(
  start: Rect,
  handle: Handle,
  delta: [number, number],
  angle: number,
  turned: Size,
  aspect: number | null,
): Rect {
  const minimum = Math.max(24, Math.min(turned.width, turned.height) * 0.03);

  const attempt = ([dx, dy]: [number, number]): Rect => {
    if (handle.x === 0 && handle.y === 0) {
      const [mx, my] = toPhoto([dx, dy], angle);
      return { ...start, x: start.x + mx, y: start.y + my };
    }
    // The crop's edges in the frame, relative to where its centre started.
    let left = -start.width / 2;
    let right = start.width / 2;
    let top = -start.height / 2;
    let bottom = start.height / 2;

    if (aspect === null) {
      if (handle.x < 0) left = Math.min(left + dx, right - minimum);
      if (handle.x > 0) right = Math.max(right + dx, left + minimum);
      if (handle.y < 0) top = Math.min(top + dy, bottom - minimum);
      if (handle.y > 0) bottom = Math.max(bottom + dy, top + minimum);
    } else {
      // Work out the new width from whichever direction the drag asks for
      // more, then let the height follow.
      const byWidth = handle.x !== 0 ? start.width + handle.x * dx : 0;
      const byHeight = handle.y !== 0 ? (start.height + handle.y * dy) * aspect : 0;
      const width = Math.max(byWidth, byHeight, minimum * Math.max(1, aspect));
      const height = width / aspect;
      // The side opposite the handle stays put; a side with no handle grows evenly.
      if (handle.x < 0) left = right - width;
      else if (handle.x > 0) right = left + width;
      else [left, right] = [-width / 2, width / 2];
      if (handle.y < 0) top = bottom - height;
      else if (handle.y > 0) bottom = top + height;
      else [top, bottom] = [-height / 2, height / 2];
    }
    const [cx, cy] = toPhoto([(left + right) / 2, (top + bottom) / 2], angle);
    return { x: start.x + cx, y: start.y + cy, width: right - left, height: bottom - top };
  };

  // Go as far along the drag as stays on the photo.
  const along = (from: [number, number], step: [number, number]): [number, number] => {
    const at = (t: number): [number, number] => [from[0] + step[0] * t, from[1] + step[1] * t];
    if (fits(attempt(at(1)), angle, turned)) return at(1);
    let low = 0;
    let high = 1;
    for (let i = 0; i < 14; i++) {
      const middle = (low + high) / 2;
      if (fits(attempt(at(middle)), angle, turned)) low = middle;
      else high = middle;
    }
    return at(low);
  };
  // First the whole drag, then whatever each axis can still give on its own,
  // so the crop slides along an edge instead of sticking to it.
  let reached = along([0, 0], delta);
  reached = along(reached, [delta[0] - reached[0], 0]);
  reached = along(reached, [0, delta[1] - reached[1]]);
  return attempt(reached);
}

/** Everything the crop tool changes. */
export type Geometry = Pick<Adjustments, (typeof GEOMETRY)[number]>;

export const geometryOf = (a: Geometry): Geometry => ({
  crop: a.crop,
  straighten: a.straighten,
  rotation: a.rotation,
  flipHorizontal: a.flipHorizontal,
  flipVertical: a.flipVertical,
});

/** Turns the picture a quarter-turn, carrying the crop round with it. */
export function quarterTurn(a: Geometry, clockwise: boolean): Geometry {
  const { x, y, width, height } = a.crop;
  return {
    rotation: (a.rotation + (clockwise ? 1 : 3)) % 4,
    // A mirror that was left-right is top-bottom once the picture is on its side.
    flipHorizontal: a.flipVertical,
    flipVertical: a.flipHorizontal,
    straighten: a.straighten,
    crop: clockwise ? { x: 1 - y, y: x, width: height, height: width } : { x: y, y: 1 - x, width: height, height: width },
  };
}

/** Mirrors the picture, and the crop and tilt with it. */
export function flipped(a: Geometry, horizontal: boolean): Geometry {
  return {
    rotation: a.rotation,
    flipHorizontal: horizontal ? !a.flipHorizontal : a.flipHorizontal,
    flipVertical: horizontal ? a.flipVertical : !a.flipVertical,
    straighten: -a.straighten,
    crop: horizontal ? { ...a.crop, x: 1 - a.crop.x } : { ...a.crop, y: 1 - a.crop.y },
  };
}

/**
 * Where the frame's centre sits on a canvas `shown` CSS pixels big that shows
 * part `region` of it (0..1 across and down), and how many CSS pixels a photo
 * pixel takes there. `frame` is the frame's size in photo pixels.
 */
export function onCanvas(frame: Size, region: Region, shown: Size): { x: number; y: number; scale: number } {
  return {
    x: ((0.5 - region.x) / region.width) * shown.width,
    y: ((0.5 - region.y) / region.height) * shown.height,
    scale: shown.width / (region.width * frame.width),
  };
}

/**
 * A shape typed by hand, as width over height: "6:7", "6x7", "65 × 24",
 * "4,5:6" or a single number like "1.5". Null if it isn't one, or is too long
 * and thin to crop to.
 */
export function parseRatio(text: string): number | null {
  const parts = text.trim().replace(/,/g, ".").split(/\s*[:x×/]\s*/i);
  if (parts.length > 2 || parts.some((part) => !/^(\d+\.?\d*|\.\d+)$/.test(part))) return null;
  const [width, height = 1] = parts.map(Number);
  const ratio = width / height;
  if (!Number.isFinite(ratio) || ratio <= 0) return null;
  return Math.max(ratio, 1 / ratio) <= 20 ? ratio : null;
}

/**
 * The crop tool's own steps, so undo works inside it without each drag
 * becoming a step in the photo's history: the whole visit is recorded as one
 * step when you're done, or none if you cancel.
 */
export interface CropSteps {
  past: Geometry[];
  now: Geometry;
  future: Geometry[];
}

/** Notes a change made in the crop tool. `start` is how the photo was framed when the tool opened. */
export function markStep(steps: CropSteps | null, start: Geometry, now: Geometry): CropSteps {
  const base = steps ?? { past: [], now: start, future: [] };
  if (same(base.now, now)) return base;
  return { past: [...base.past, base.now], now, future: [] };
}

/**
 * What the history calls a visit to the crop tool: one change keeps its own
 * name ("Rotate", "Straighten 2.0°"), several are simply "Crop", and
 * putting everything back is a reset.
 */
export function visitName(steps: CropSteps | null, before: Adjustments, after: Adjustments): string {
  const name = describeChange(before, after);
  return steps && steps.past.length > 1 && !name.startsWith("Reset") ? "Crop" : name;
}

/** One change back, or null if there is nothing left to take back in the crop tool. */
export function undoStep(steps: CropSteps | null): CropSteps | null {
  if (!steps || steps.past.length === 0) return null;
  return { past: steps.past.slice(0, -1), now: steps.past[steps.past.length - 1], future: [steps.now, ...steps.future] };
}

/** One change forward again, or null if there is none. */
export function redoStep(steps: CropSteps | null): CropSteps | null {
  if (!steps || steps.future.length === 0) return null;
  return { past: [...steps.past, steps.now], now: steps.future[0], future: steps.future.slice(1) };
}
