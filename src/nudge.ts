/**
 * Moving things on the photo with the arrow keys: a handle with the focus
 * moves a pixel on screen at a time, or ten with Shift, so it is as fine as
 * the zoom.
 */

export const NUDGE = 1;
export const BIG_NUDGE = 10;

/** Where an arrow key moves something on screen, in pixels (y runs down), or null for any other key. */
export function arrowOffset(key: string, big: boolean): [number, number] | null {
  const step = big ? BIG_NUDGE : NUDGE;
  switch (key) {
    case "ArrowLeft":
      return [-step, 0];
    case "ArrowRight":
      return [step, 0];
    case "ArrowUp":
      return [0, -step];
    case "ArrowDown":
      return [0, step];
    default:
      return null;
  }
}
