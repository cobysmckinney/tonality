// The command palette's model: what a command is, which ones are offered right now, how a
// query ranks them, and how their shortcuts read. No app state here, so it can be tested.
import { fuzzy } from "./match";

export const GROUPS = ["Library", "Edit", "Crop", "Masks", "History", "View", "App"] as const;
export type Group = (typeof GROUPS)[number];

export interface Command {
  id: string;
  title: string;
  group: Group;
  /**
   * The shortcuts that do the same thing, each like "Mod+Shift+E" (Mod is Ctrl, or ⌘ on
   * macOS). Shown in the palette and in the Shortcuts list.
   */
  keys?: string[];
  /** Other words it can be found by ("preferences" for Settings). */
  keywords?: string;
  /** Can it run now (the mode, a photo open, a selection…)? Unavailable commands are hidden. */
  when?: () => boolean;
  /** Without one it's a shortcut only, listed in Shortcuts but not offered as a command. */
  run?: () => unknown;
}

export interface Ranked {
  command: Command;
  /** Characters of the title the query matched. */
  hits: number[];
}

/** Commands that can run right now, in registry order. */
export function available(commands: readonly Command[]): Command[] {
  return commands.filter((c) => c.run && (c.when?.() ?? true));
}

/**
 * Rank `commands` for `query`. Every word of the query has to match the title, the group or
 * the keywords; titles count most. An empty query keeps the order, with `recent` ids first.
 */
export function search(commands: readonly Command[], query: string, recent: string[]): Ranked[] {
  const words = query.trim().split(/\s+/).filter(Boolean);
  const recency = (id: string) => {
    const i = recent.indexOf(id);
    return i < 0 ? 0 : recent.length - i;
  };
  if (!words.length) {
    const order = new Map(commands.map((c, i) => [c.id, i]));
    return [...commands]
      .sort((a, b) => recency(b.id) - recency(a.id) || order.get(a.id)! - order.get(b.id)!)
      .map((command) => ({ command, hits: [] }));
  }
  const out: (Ranked & { score: number; i: number })[] = [];
  commands.forEach((command, i) => {
    let score = 0;
    const hits = new Set<number>();
    for (const w of words) {
      const t = fuzzy(w, command.title);
      const g = fuzzy(w, command.group);
      const k = command.keywords ? fuzzy(w, command.keywords) : null;
      const other = Math.max(g?.score ?? -Infinity, k?.score ?? -Infinity) / 2;
      if (t && t.score >= other) {
        score += t.score;
        for (const h of t.hits) hits.add(h);
      } else if (other > -Infinity) score += other;
      else return;
    }
    // Recently used breaks near-ties, without lifting a poor match over a good one.
    score += Math.min(recency(command.id), 5) * 0.5;
    out.push({ command, hits: [...hits].sort((a, b) => a - b), score, i });
  });
  return out
    .sort((a, b) => b.score - a.score || a.i - b.i)
    .map(({ command, hits }) => ({ command, hits }));
}

/** Move `id` to the front of the recently used list. */
export function remember(recent: string[], id: string, max = 8): string[] {
  return [id, ...recent.filter((r) => r !== id)].slice(0, max);
}

/** Every command with a shortcut, by group in `GROUPS` order (whether or not it can run now). */
export function shortcutGroups(commands: readonly Command[]): { group: Group; items: Command[] }[] {
  return GROUPS.map((group) => ({
    group,
    items: commands.filter((c) => c.group === group && c.keys?.length),
  })).filter((g) => g.items.length);
}

const NAMES: Record<string, [win: string, mac: string]> = {
  Mod: ["Ctrl", "⌘"],
  Shift: ["Shift", "⇧"],
  Alt: ["Alt", "⌥"],
  Enter: ["Enter", "↩"],
  Esc: ["Esc", "Esc"],
  Delete: ["Delete", "⌫"],
  Left: ["←", "←"],
  Right: ["→", "→"],
  Arrows: ["Arrows", "Arrows"],
};

/** Reversed, so a key that isn't a modifier (-1) sorts last. */
const MAC_ORDER = ["Mod", "Shift", "Alt"];

/** The keys of one shortcut ("Mod+Shift+E") as they're labelled on this platform. */
export function keyLabels(combo: string, mac: boolean): string[] {
  // "+" alone (or at the end, "Mod++") is the plus key, not a separator.
  const parts = combo === "+" ? ["+"] : combo.replace(/\+\+$/, "+plus").split("+");
  // macOS writes modifiers in the order ⌥⇧⌘.
  if (mac) parts.sort((a, b) => MAC_ORDER.indexOf(b) - MAC_ORDER.indexOf(a));
  return parts.map((p) => (p === "plus" ? "+" : (NAMES[p]?.[mac ? 1 : 0] ?? p)));
}

/** One shortcut as a single string, for tooltips: "Ctrl+Shift+E", or "⇧⌘E" on macOS. */
export function shortcutText(combo: string, mac: boolean): string {
  return keyLabels(combo, mac).join(mac ? "" : "+");
}
