/**
 * Rows of tabs (the tool rail, the curve's channels, the mixer's colours).
 * Tab reaches the chosen tab; the arrow keys, Home and End move between tabs
 * and choose as they go, as tabs do elsewhere.
 */

/**
 * The tab a key moves to from tab `at` of `count`, going round at the ends and
 * passing over tabs that can't be chosen; null for keys that don't move, or
 * when no tab can be chosen.
 */
export function tabAfter(key: string, at: number, count: number, usable: (index: number) => boolean = () => true): number | null {
  if (count === 0) return null;
  let start: number;
  let step: number;
  if (key === "ArrowRight" || key === "ArrowDown") [start, step] = [at + 1, 1];
  else if (key === "ArrowLeft" || key === "ArrowUp") [start, step] = [at - 1, -1];
  else if (key === "Home") [start, step] = [0, 1];
  else if (key === "End") [start, step] = [count - 1, -1];
  else return null;
  for (let i = 0; i < count; i++) {
    const index = (((start + i * step) % count) + count) % count;
    if (usable(index)) return index;
  }
  return null;
}

/**
 * For a tab list's keydown: moves the focus to the next tab that can be chosen, and chooses it.
 * The key stops there, so Left and Right on a tab don't also change photo.
 */
export function onTabKey(
  event: { key: string; currentTarget: HTMLElement; preventDefault: () => void; stopPropagation: () => void },
  choose: (index: number) => void,
) {
  const tabs = [...event.currentTarget.querySelectorAll<HTMLElement>('[role="tab"]')];
  const at = tabs.findIndex((tab) => tab === document.activeElement);
  const usable = (index: number) => !tabs[index].matches(":disabled, [aria-disabled='true']");
  const next = tabAfter(event.key, Math.max(at, 0), tabs.length, usable);
  if (next === null) return;
  event.preventDefault();
  event.stopPropagation();
  choose(next);
  tabs[next].focus();
}
