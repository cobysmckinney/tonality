import { describe, expect, it } from "vitest";
import { fuzzy } from "./match";

describe("fuzzy", () => {
  it("needs every letter, in order", () => {
    expect(fuzzy("exp", "Export…")).not.toBeNull();
    expect(fuzzy("xe", "Export…")).toBeNull();
    expect(fuzzy("exports", "Export")).toBeNull();
  });

  it("ignores case", () => {
    expect(fuzzy("UNDO", "Undo")?.hits).toEqual([0, 1, 2, 3]);
  });

  it("matches everything with an empty query", () => {
    expect(fuzzy("", "Anything")).toEqual({ score: 0, hits: [] });
  });

  it("prefers the starts of words", () => {
    // "if" could be the i and f of "Import"…, but the word starts win.
    expect(fuzzy("if", "Import folder…")?.hits).toEqual([0, 7]);
    expect(fuzzy("zi", "Zoom in")!.score).toBeGreaterThan(
      fuzzy("zi", "Fit zoom to window")?.score ?? 0,
    );
  });

  it("prefers letters that run together", () => {
    expect(fuzzy("crop", "Crop")!.score).toBeGreaterThan(
      fuzzy("crop", "Cancel rotation op")!.score,
    );
  });

  it("scores an earlier match above a later one", () => {
    expect(fuzzy("mask", "Masks")!.score).toBeGreaterThan(fuzzy("mask", "Delete mask")!.score);
  });

  it("reports the matched characters", () => {
    expect(fuzzy("nxp", "Next photo")?.hits).toEqual([0, 2, 5]);
  });
});
