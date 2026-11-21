// Grid selection: which photos are selected, the keyboard focus, and the range anchor.
// Pure so it can be tested; LibraryState passes itself (its `selected` is a SvelteSet).

export interface Selection {
  selected: Set<number>;
  /** Keyboard focus (image id): where the arrow keys move from. */
  focus: number | null;
  /** Where a Shift range starts. Moves only on a plain or Ctrl click, or a plain arrow key. */
  anchor: number | null;
}

export interface Mods {
  shiftKey?: boolean;
  ctrlKey?: boolean;
  metaKey?: boolean;
}

/** Ids from `a` to `b` inclusive, in list order; null if either isn't listed. */
export function rangeIds(ids: readonly number[], a: number, b: number): number[] | null {
  const i = ids.indexOf(a);
  const j = ids.indexOf(b);
  if (i < 0 || j < 0) return null;
  return ids.slice(Math.min(i, j), Math.max(i, j) + 1);
}

/** Select from the anchor to `id`. Without `add`, the range replaces the selection. */
function extendTo(s: Selection, ids: readonly number[], id: number, add: boolean): boolean {
  const range = rangeIds(ids, s.anchor ?? s.focus ?? id, id);
  if (!range) return false;
  if (s.anchor === null) s.anchor = s.focus ?? id;
  if (!add) s.selected.clear();
  for (const x of range) s.selected.add(x);
  s.focus = id;
  return true;
}

/** A click on `id`: Shift selects a range from the anchor, Ctrl/Cmd toggles, else just it. */
export function clickSelect(s: Selection, ids: readonly number[], id: number, e?: Mods) {
  const add = !!(e?.ctrlKey || e?.metaKey);
  if (e?.shiftKey && (s.anchor ?? s.focus) !== null && extendTo(s, ids, id, add)) return;
  if (add) {
    if (s.selected.has(id)) s.selected.delete(id);
    else s.selected.add(id);
  } else {
    s.selected.clear();
    s.selected.add(id);
  }
  s.focus = s.anchor = id;
}

/** An arrow key moved focus to `id`. With Shift, the selection becomes anchor..id. */
export function arrowSelect(s: Selection, ids: readonly number[], id: number, extend: boolean) {
  if (extend && extendTo(s, ids, id, false)) return;
  clickSelect(s, ids, id);
}
