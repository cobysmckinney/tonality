/**
 * Comparing the photo with how it was: the before side of the editor's
 * split view. Kept out of the store so it can be tested.
 */
import { Adjustments, DEFAULTS, GEOMETRY } from "./adjustments";

/** What the before side shows: the original, or the photo as of an earlier step. */
export interface CompareWith {
  /** The step, by id and name; null is the original. */
  step: { id: number; label: string } | null;
  /** Its recipe. */
  adjustments: Adjustments;
}

export const ORIGINAL: CompareWith = { step: null, adjustments: DEFAULTS };

/**
 * The recipe the before side is drawn with: the look it is compared with,
 * framed the way the photo is now (crop, straighten, turns and flips), so
 * the two sides line up across the split.
 */
export function beforeRecipe(before: Adjustments, now: Adjustments): Adjustments {
  const framed: Partial<Adjustments> = {};
  for (const key of GEOMETRY) (framed as Record<string, unknown>)[key] = now[key];
  return { ...before, ...framed };
}

/** The split's position across the photo, 0 (all after) to 1 (all before), for a pointer at `clientX` over a box. */
export const splitAt = (clientX: number, box: { left: number; width: number }) =>
  box.width > 0 ? Math.min(1, Math.max(0, (clientX - box.left) / box.width)) : 0.5;

/** What the before side is called on the photo. */
export const beforeLabel = (compare: CompareWith) => (compare.step ? `Step: ${compare.step.label}` : "Original");
