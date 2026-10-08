/**
 * Keeps the grid's selection, cursor and open photo pointing at photos that
 * are still on screen when some leave it: moved to Recently Deleted, taken
 * out of an album, or hidden by the flag filter.
 */

export interface Pointers {
  selection: Set<number>;
  /** Where a Shift-click range starts. */
  anchor: number | null;
  /** The photo the arrow keys move from. */
  cursor: number | null;
  /** The photo open in the viewer. */
  openId: number | null;
}

/** The nearest photo after `id` that stays, else the nearest before it. */
function neighbor(visible: number[], gone: Set<number>, id: number): number | null {
  const at = visible.indexOf(id);
  if (at < 0) return null;
  const next = visible.slice(at + 1).find((other) => !gone.has(other));
  const previous = visible.slice(0, at).reverse().find((other) => !gone.has(other));
  return next ?? previous ?? null;
}

/**
 * Where the pointers go when the photos in `gone` leave `visible` (the grid's
 * order before they left). A cursor or open photo that left steps to its
 * nearest neighbour, the next one first. If that empties the selection, the
 * neighbour is selected instead, so the arrows and the next keypress carry on
 * from where you were.
 */
export function forget(visible: number[], gone: Set<number>, pointers: Pointers): Pointers {
  const { cursor, anchor, openId } = pointers;
  const selection = new Set([...pointers.selection].filter((id) => !gone.has(id)));
  const stepped = cursor !== null && gone.has(cursor) ? neighbor(visible, gone, cursor) : cursor;
  const result: Pointers = {
    selection,
    anchor: anchor !== null && gone.has(anchor) ? null : anchor,
    cursor: stepped,
    openId: openId !== null && gone.has(openId) ? neighbor(visible, gone, openId) : openId,
  };
  const lostSelected = cursor !== null && gone.has(cursor) && pointers.selection.has(cursor);
  if (lostSelected && selection.size === 0 && stepped !== null) {
    result.selection = new Set([stepped]);
    result.anchor = stepped;
  }
  return result;
}
