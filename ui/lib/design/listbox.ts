// Keyboard behaviour of a single-select listbox (`Select.svelte`), kept apart so it can be tested.

/** The option `key` moves to from `i` among `n`, or null if the key doesn't move. No wrapping. */
export function moveTo(key: string, i: number, n: number): number | null {
  if (n === 0) return null;
  switch (key) {
    case "ArrowDown":
      return Math.min(n - 1, i + 1);
    case "ArrowUp":
      return Math.max(0, i - 1);
    case "Home":
    case "PageUp":
      return 0;
    case "End":
    case "PageDown":
      return n - 1;
    default:
      return null;
  }
}

/** How long typed letters keep adding to one search. */
export const TYPEAHEAD_MS = 600;

/**
 * Type-ahead: the option whose label starts with `typed`, searching from the one after `from`
 * and wrapping. Typing one letter again and again (`"ccc"`) steps through the labels starting
 * with it. Case-insensitive; null if nothing matches.
 */
export function typeahead(labels: readonly string[], typed: string, from: number): number | null {
  const n = labels.length;
  if (!n || !typed) return null;
  let q = typed.toLowerCase();
  const same = [...q].every((c) => c === q[0]);
  if (same) q = q[0];
  // A longer search may still match the current option; a repeated letter moves on from it.
  const start = same ? from + 1 : q.length > 1 ? from : from + 1;
  for (let k = 0; k < n; k++) {
    const i = (((start + k) % n) + n) % n;
    if (labels[i].toLowerCase().startsWith(q)) return i;
  }
  return null;
}
