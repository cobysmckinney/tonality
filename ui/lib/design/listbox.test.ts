import { describe, expect, it } from "vitest";
import { moveTo, typeahead } from "./listbox";

const modes = [
  "As Shot",
  "Auto",
  "Daylight",
  "Cloudy",
  "Shade",
  "Tungsten",
  "Fluorescent",
  "Flash",
  "Custom",
];

describe("moveTo", () => {
  it("steps without wrapping", () => {
    expect(moveTo("ArrowDown", 0, 3)).toBe(1);
    expect(moveTo("ArrowDown", 2, 3)).toBe(2);
    expect(moveTo("ArrowUp", 0, 3)).toBe(0);
    expect(moveTo("ArrowUp", 2, 3)).toBe(1);
  });
  it("jumps to the ends", () => {
    expect(moveTo("Home", 2, 5)).toBe(0);
    expect(moveTo("End", 0, 5)).toBe(4);
  });
  it("ignores other keys and empty lists", () => {
    expect(moveTo("a", 0, 3)).toBeNull();
    expect(moveTo("ArrowDown", 0, 0)).toBeNull();
  });
});

describe("typeahead", () => {
  it("finds the next label starting with the typed letters", () => {
    expect(typeahead(modes, "d", 0)).toBe(2);
    expect(typeahead(modes, "T", 0)).toBe(5);
    expect(typeahead(modes, "fla", 0)).toBe(7);
  });
  it("steps through labels when one letter is repeated", () => {
    // C: Cloudy, then Custom, then back to Cloudy.
    expect(typeahead(modes, "c", 0)).toBe(3);
    expect(typeahead(modes, "cc", 3)).toBe(8);
    expect(typeahead(modes, "ccc", 8)).toBe(3);
  });
  it("keeps the current option while a longer search still matches it", () => {
    expect(typeahead(modes, "au", 1)).toBe(1);
    expect(typeahead(modes, "as", 1)).toBe(0);
  });
  it("returns null when nothing matches", () => {
    expect(typeahead(modes, "x", 0)).toBeNull();
    expect(typeahead(modes, "", 0)).toBeNull();
    expect(typeahead([], "a", 0)).toBeNull();
  });
});
