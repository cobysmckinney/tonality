/**
 * Rows of tabs (the tool rail, the curve's channels, the mixer's colours).
 * Tab reaches the chosen tab; the arrow keys, Home and End move between tabs
 * and choose as they go, as tabs do elsewhere.
 */

/** The tab a key moves to from tab `at` of `count`, going round at the ends; null for keys that don't move. */
export function tabAfter(key: string, at: number, count: number): number | null {
  if (count === 0) return null;
  if (key === "ArrowRight" || key === "ArrowDown") return (at + 1) % count;
  if (key === "ArrowLeft" || key === "ArrowUp") return (at - 1 + count) % count;
  if (key === "Home") return 0;
  if (key === "End") return count - 1;
  return null;
}

/**
 * For a tab list's keydown: moves the focus to the next tab and chooses it.
 * The key stops there, so Left and Right on a tab don't also change photo.
 */
export function onTabKey(
  event: { key: string; currentTarget: HTMLElement; preventDefault: () => void; stopPropagation: () => void },
  choose: (index: number) => void,
) {
  const tabs = [...event.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]')];
  const at = tabs.findIndex((tab) => tab === document.activeElement);
  const next = tabAfter(event.key, Math.max(at, 0), tabs.length);
  if (next === null) return;
  event.preventDefault();
  event.stopPropagation();
  choose(next);
  tabs[next].focus();
}
