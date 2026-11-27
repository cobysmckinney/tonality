// The shortcut sheet (?) is built from `COMMANDS`, but the keys themselves are handled in the
// components, and `COMMANDS` only names them: a hand-kept copy that has drifted before (W and
// Space were missing). This reads the handlers' sources and checks every letter key they act
// on, and Space, has an entry.
import { describe, expect, it } from "vitest";
import app from "../../App.svelte?raw";
import canvas from "../edit/Canvas.svelte?raw";
import edit from "../edit/Edit.svelte?raw";
import library from "../library/Library.svelte?raw";
import commands from "./commands.svelte.ts?raw";

/** Keys the commands name: the last part of each combo, upper case. Read from the `keys: [...]`
 *  lists and from `get keys()` bodies (a command whose keys depend on the mode). */
function listed(): Set<string> {
  const out = new Set<string>();
  const lists = [
    ...commands.matchAll(/keys:\s*\[([^\]]*)\]/g),
    ...commands.matchAll(/get keys\(\)\s*\{([^}]*)\}/g),
  ];
  for (const [, list] of lists) {
    for (const [, combo] of list.matchAll(/"((?:[^"\\]|\\.)*)"/g))
      out.add(combo.split("+").at(-1)!.toUpperCase());
  }
  return out;
}

/** Letter keys (and Space) a component's key handler compares `e.key` with. */
function handled(text: string): string[] {
  const keys = new Set<string>();
  for (const [, key] of text.matchAll(/e\.key(?:\.toLowerCase\(\))? === "([a-z ])"/g))
    keys.add(key === " " ? "SPACE" : key.toUpperCase());
  return [...keys].sort();
}

describe("the shortcut sheet", () => {
  const names = listed();
  for (const [file, text] of Object.entries({
    App: app,
    Library: library,
    Edit: edit,
    Canvas: canvas,
  })) {
    it(`lists every letter key ${file}.svelte handles`, () => {
      const keys = handled(text);
      expect(keys.length).toBeGreaterThan(0);
      expect(keys.filter((k) => !names.has(k))).toEqual([]);
    });
  }

  it("names the two that were missing", () => {
    expect(names.has("W")).toBe(true);
    expect(names.has("SPACE")).toBe(true);
  });
});
