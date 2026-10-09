import { describe, expect, test } from "bun:test";
import { firesShutter, opensShortcuts, SHORTCUTS } from "./keys";

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

describe("taking a photo from the keyboard", () => {
  const capturing = { ...nothing, shortcutsOpen: false, capture: { follow: true } };

  test("F12 fires the shutter while a camera is capturing", () => {
    expect(firesShutter("F12", capturing)).toBe(true);
    expect(firesShutter("F11", capturing)).toBe(false);
    expect(firesShutter("F12", { ...capturing, capture: null })).toBe(false);
  });

  test("not over a menu, a question, a sheet or the list of shortcuts", () => {
    expect(firesShutter("F12", { ...capturing, menu: { x: 0, y: 0, entries: [] } })).toBe(false);
    expect(firesShutter("F12", { ...capturing, confirmRequest: { title: "Delete?" } })).toBe(false);
    expect(firesShutter("F12", { ...capturing, importState: { phase: "review" } })).toBe(false);
    expect(firesShutter("F12", { ...capturing, exportState: { phase: "setup" } })).toBe(false);
    expect(firesShutter("F12", { ...capturing, filmSheet: { photos: [] } })).toBe(false);
    expect(firesShutter("F12", { ...capturing, shortcutsOpen: true })).toBe(false);
  });
});
