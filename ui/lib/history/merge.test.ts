import { describe, expect, it } from "vitest";
import { orderedPanels, parsePanels } from "./merge";

describe("parsePanels", () => {
  it("restores a remembered choice", () => {
    expect([...parsePanels('["light","color"]')]).toEqual(["light", "color"]);
  });

  it("drops panels it doesn't know, and falls back to Color", () => {
    expect([...parsePanels('["light","sparkle"]')]).toEqual(["light"]);
    expect([...parsePanels('["sparkle"]')]).toEqual(["color"]);
    expect([...parsePanels(null)]).toEqual(["color"]);
    expect([...parsePanels("{not json")]).toEqual(["color"]);
    expect([...parsePanels("[]")]).toEqual(["color"]);
  });
});

describe("orderedPanels", () => {
  it("lists the choice in the dialog's order, whatever order it was ticked in", () => {
    expect(orderedPanels(new Set(["masks", "light", "whiteBalance"]))).toEqual([
      "whiteBalance",
      "light",
      "masks",
    ]);
  });
});
