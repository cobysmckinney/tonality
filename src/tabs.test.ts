import { describe, expect, test } from "bun:test";
import { tabAfter } from "./tabs";

describe("moving along a row of tabs", () => {
  test("the arrow keys move one tab either way, in a row or a column", () => {
    expect(tabAfter("ArrowRight", 1, 4)).toBe(2);
    expect(tabAfter("ArrowDown", 1, 4)).toBe(2);
    expect(tabAfter("ArrowLeft", 1, 4)).toBe(0);
    expect(tabAfter("ArrowUp", 1, 4)).toBe(0);
  });

  test("they go round at the ends", () => {
    expect(tabAfter("ArrowRight", 3, 4)).toBe(0);
    expect(tabAfter("ArrowLeft", 0, 4)).toBe(3);
  });

  test("Home and End go to the first and last tab", () => {
    expect(tabAfter("Home", 2, 4)).toBe(0);
    expect(tabAfter("End", 0, 4)).toBe(3);
  });

  test("other keys, or no tabs, don't move", () => {
    expect(tabAfter("Enter", 1, 4)).toBeNull();
    expect(tabAfter("a", 1, 4)).toBeNull();
    expect(tabAfter("ArrowRight", 0, 0)).toBeNull();
  });

  test("they pass over tabs that can't be chosen", () => {
    // Only the last of four tabs can be chosen, as in the rail of a photo that can't be opened.
    const onlyLast = (index: number) => index === 3;
    expect(tabAfter("ArrowDown", 3, 4, onlyLast)).toBe(3);
    expect(tabAfter("ArrowUp", 3, 4, onlyLast)).toBe(3);
    expect(tabAfter("Home", 3, 4, onlyLast)).toBe(3);
    expect(tabAfter("End", 3, 4, onlyLast)).toBe(3);
    // The second can't be chosen: moving steps over it either way, and Home and End look inwards.
    const notSecond = (index: number) => index !== 1;
    expect(tabAfter("ArrowRight", 0, 4, notSecond)).toBe(2);
    expect(tabAfter("ArrowLeft", 2, 4, notSecond)).toBe(0);
    expect(tabAfter("Home", 2, 4, (index) => index > 1)).toBe(2);
    expect(tabAfter("End", 0, 4, (index) => index < 2)).toBe(1);
  });

  test("with none that can be chosen, nothing moves", () => {
    expect(tabAfter("ArrowRight", 0, 4, () => false)).toBeNull();
  });
});
