import { Adjustments, Point } from "./adjustments";
import { Size } from "./crop";
import { apply, uncroppedToSource } from "./masks";

/**
 * Where a click lands on the photo file (0..1 across and down), when the
 * photo is shown uncropped as the film base picker shows it: `at` is the
 * click on the shown picture in CSS pixels, `shown` that picture's size, and
 * `region` the part of the uncropped frame it shows.
 */
export function pickedPoint(
  photo: Size,
  adjustments: Adjustments,
  region: { x: number; y: number; width: number; height: number },
  shown: Size,
  [x, y]: Point,
): Point {
  const frame: Point = [region.x + (x / shown.width) * region.width, region.y + (y / shown.height) * region.height];
  return apply(uncroppedToSource(photo, adjustments), frame);
}

/** A colour in sRGB levels (0..255) as CSS. */
export const cssColour = ([r, g, b]: [number, number, number]) => `rgb(${r} ${g} ${b})`;
