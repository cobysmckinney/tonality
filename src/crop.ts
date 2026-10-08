/**
 * Crop arithmetic for the crop tool. Mirrors src-tauri/src/geometry.rs, which
 * is what actually draws the picture: the two must agree on what a crop means.
 *
 * Everything here is measured on the "turned" photo (after quarter-turns and
 * flips), in its pixels. The crop is an upright rectangle in a frame tilted
 * by the straighten angle; "frame" directions are what you see on screen in
 * the crop tool, "photo" directions are along the turned photo's own edges.
 */
import type { Adjustments, Crop } from "./adjustments";

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

type Geometry = Pick<Adjustments, "crop" | "straighten" | "rotation" | "flipHorizontal" | "flipVertical">;

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
