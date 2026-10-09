import { Adjustments, Point } from "./adjustments";
import { Size } from "./crop";
import { apply, frameToSource, uncroppedToSource } from "./masks";

/**
 * Where a click lands on the photo file (0..1 across and down): `at` is the
 * click on the shown picture in CSS pixels, `shown` that picture's size, and
 * `region` the part of the frame it shows. The frame is the whole photo
 * uncropped, as the film base picker shows it, unless `uncropped` is false.
 */
export function pickedPoint(
  photo: Size,
  adjustments: Adjustments,
  region: { x: number; y: number; width: number; height: number },
  shown: Size,
  [x, y]: Point,
  uncropped = true,
): Point {
  const frame: Point = [region.x + (x / shown.width) * region.width, region.y + (y / shown.height) * region.height];
  return apply((uncropped ? uncroppedToSource : frameToSource)(photo, adjustments), frame);
}

/** A colour in sRGB levels (0..255) as CSS. */
export const cssColour = ([r, g, b]: [number, number, number]) => `rgb(${r} ${g} ${b})`;
