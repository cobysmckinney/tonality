import { describe, expect, test } from "bun:test";
import { opensShortcuts, SHORTCUTS } from "./keys";

const nothing = { menu: null, confirmRequest: null, importState: null, exportState: null, filmSheet: null };

describe("the list of shortcuts", () => {
  test("opens with ?", () => {
    expect(opensShortcuts("?", false, nothing)).toBe(true);
    expect(opensShortcuts("/", false, nothing)).toBe(false);
  });

  test("doesn't open while typing, or over a menu, a question or a sheet", () => {
    expect(opensShortcuts("?", true, nothing)).toBe(false);
    expect(opensShortcuts("?", false, { ...nothing, menu: { x: 0, y: 0, entries: [] } })).toBe(false);
    expect(opensShortcuts("?", false, { ...nothing, confirmRequest: { title: "Delete?" } })).toBe(false);
    expect(opensShortcuts("?", false, { ...nothing, importState: { phase: "review" } })).toBe(false);
    expect(opensShortcuts("?", false, { ...nothing, exportState: { phase: "setup" } })).toBe(false);
    expect(opensShortcuts("?", false, { ...nothing, filmSheet: { photos: [] } })).toBe(false);
  });

  test("says what each key does once per group", () => {
    for (const group of SHORTCUTS) {
      const said = group.shortcuts.map((shortcut) => shortcut.does);
      expect(new Set(said).size).toBe(said.length);
    }
  });

  test("names each key once per group", () => {
    for (const group of SHORTCUTS) {
      const keys = group.shortcuts.flatMap((shortcut) => shortcut.keys);
      expect(new Set(keys).size).toBe(keys.length);
    }
  });
});
