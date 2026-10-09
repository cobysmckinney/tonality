import { describe, expect, test } from "bun:test";
import { DEFAULTS, defaultAdjustments } from "./adjustments";
import { beforeLabel, beforeRecipe, ORIGINAL, splitAt } from "./compare";

const edited = {
  ...defaultAdjustments(),
  exposure: 1.5,
  saturation: -40,
  vignette: 30,
  crop: { x: 0.4, y: 0.6, width: 0.5, height: 0.5 },
  straighten: 3,
  rotation: 1,
  flipHorizontal: true,
  flipVertical: false,
};

describe("the before side of a comparison", () => {
  test("the original is drawn as shot, but framed like the edit so the sides line up", () => {
    const before = beforeRecipe(DEFAULTS, edited);
    expect(before.exposure).toBe(0);
    expect(before.saturation).toBe(0);
    expect(before.vignette).toBe(0);
    expect(before.crop).toEqual(edited.crop);
    expect([before.straighten, before.rotation, before.flipHorizontal, before.flipVertical]).toEqual([3, 1, true, false]);
  });

  test("an earlier step keeps its own look and takes the current framing", () => {
    const step = { ...defaultAdjustments(), exposure: 0.5, crop: { x: 0.5, y: 0.5, width: 1, height: 1 }, rotation: 2 };
    const before = beforeRecipe(step, edited);
    expect(before.exposure).toBe(0.5);
    expect(before.rotation).toBe(1);
    expect(before.crop).toEqual(edited.crop);
  });

  test("is named after what it shows", () => {
    expect(beforeLabel(ORIGINAL)).toBe("Original");
    expect(beforeLabel({ step: { id: 3, label: "Exposure +0.50" }, adjustments: DEFAULTS })).toBe("Step: Exposure +0.50");
  });
});

describe("dragging the split", () => {
  const box = { left: 100, width: 400 };
  test("follows the pointer across the photo", () => {
    expect(splitAt(300, box)).toBe(0.5);
    expect(splitAt(200, box)).toBe(0.25);
  });
  test("stops at the photo's edges", () => {
    expect(splitAt(0, box)).toBe(0);
    expect(splitAt(900, box)).toBe(1);
  });
  test("stays put on a box with no width", () => {
    expect(splitAt(300, { left: 0, width: 0 })).toBe(0.5);
  });
});
