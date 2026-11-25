// Small pieces of rating, flagging and the "edited" mark, kept apart so they can be tested.

/** Structural equality of two JSON values (edit states as the backend sends them). */
export function sameJson(a: unknown, b: unknown): boolean {
  if (a === b) return true;
  if (typeof a !== "object" || typeof b !== "object" || a === null || b === null) return false;
  if (Array.isArray(a) !== Array.isArray(b)) return false;
  if (Array.isArray(a)) {
    const bs = b as unknown[];
    return a.length === bs.length && a.every((v, i) => sameJson(v, bs[i]));
  }
  const ao = a as Record<string, unknown>;
  const bo = b as Record<string, unknown>;
  const keys = Object.keys(ao);
  return (
    keys.length === Object.keys(bo).length && keys.every((k) => k in bo && sameJson(ao[k], bo[k]))
  );
}

/** The rating a key gives: pressing the photos' current rating again clears it. */
export function nextRating(current: (number | undefined)[], pressed: number): number {
  return current.length && current.every((r) => r === pressed) ? 0 : pressed;
}

/** What the canvas briefly shows after a rating key: "★★★", or "No rating". */
export function ratingNote(rating: number): string {
  return rating > 0 ? "★".repeat(rating) : "No rating";
}

/** What the canvas briefly shows after a flag key. */
export function flagNote(flag: number): string {
  return flag > 0 ? "Picked" : flag < 0 ? "Rejected" : "Unflagged";
}
