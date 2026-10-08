import { describe, expect, test } from "bun:test";
import { CurvePoints } from "./adjustments";
import { addPointNear, movePoint, nudgePoint, removePoint } from "./curve";
import { arrowOffset } from "./nudge";

const straight: CurvePoints = [
  [0, 0],
  [1, 1],
];
const bent: CurvePoints = [
  [0, 0],
  [0.5, 0.6],
  [1, 1],
];

describe("a curve's points", () => {
  test("stay between their neighbours, and the ends stay at the edges", () => {
    expect(movePoint(bent, 1, [0.99, 0.5])[1][0]).toBeCloseTo(0.98, 6);
    expect(movePoint(bent, 0, [0.3, 0.2])[0]).toEqual([0, 0.2]);
    expect(movePoint(bent, 2, [0.3, 1.4])[2]).toEqual([1, 1]);
  });

  test("go when removed, except the ends, which go back to their corners", () => {
    expect(removePoint(bent, 1)).toEqual(straight);
    expect(removePoint([[0, 0.3], [1, 0.8]], 0)).toEqual([[0, 0], [1, 0.8]]);
    expect(removePoint([[0, 0.3], [1, 0.8]], 1)).toEqual([[0, 0.3], [1, 1]]);
  });
});

describe("the arrow keys on a curve point", () => {
  test("move it up and down by 1%, or 5% with Shift", () => {
    expect(nudgePoint(bent, 1, "ArrowUp", false)![1]).toEqual([0.5, 0.61]);
    expect(nudgePoint(bent, 1, "ArrowDown", true)![1]).toEqual([0.5, 0.55]);
  });

  test("move it across, but not the ends", () => {
    expect(nudgePoint(bent, 1, "ArrowRight", false)![1]).toEqual([0.51, 0.6]);
    expect(nudgePoint(bent, 0, "ArrowRight", false)![0]).toEqual([0, 0]);
  });

  test("leave other keys alone", () => {
    expect(nudgePoint(bent, 1, "Enter", false)).toBeNull();
  });
});

describe("adding a point from the keyboard", () => {
  test("puts it on the curve, halfway to the next point", () => {
    const added = addPointNear(straight, 0)!;
    expect(added.index).toBe(1);
    expect(added.points).toEqual([[0, 0], [0.5, 0.5], [1, 1]]);
  });

  test("from the last point, halfway back to the one before", () => {
    const added = addPointNear(bent, 2)!;
    expect(added.index).toBe(2);
    expect(added.points[2][0]).toBe(0.75);
    expect(added.points).toHaveLength(4);
  });

  test("doesn't squeeze one in where there's no room", () => {
    expect(addPointNear([[0, 0], [0.5, 0.5], [0.53, 0.6], [1, 1]], 1)).toBeNull();
  });
});

describe("an arrow key on a handle", () => {
  test("moves it a pixel on screen, or ten with Shift", () => {
    expect(arrowOffset("ArrowLeft", false)).toEqual([-1, 0]);
    expect(arrowOffset("ArrowDown", true)).toEqual([0, 10]);
    expect(arrowOffset("a", false)).toBeNull();
  });
});
