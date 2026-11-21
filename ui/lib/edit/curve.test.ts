import { describe, expect, it } from "vitest";
import { curve, dragPoint, isIdentity, normalise, type Point } from "./curve";

describe("curve", () => {
  it("is the identity for degenerate input", () => {
    for (const pts of [
      [],
      [[0.3, 0.7]],
      [
        [0, 0],
        [1, 1],
      ],
    ] as [number, number][][]) {
      const f = curve(pts);
      for (const x of [0, 0.25, 0.5, 1]) expect(f(x)).toBeCloseTo(x, 6);
    }
    expect(isIdentity([])).toBe(true);
  });

  it("passes through points and stays monotone", () => {
    const pts: [number, number][] = [
      [0, 0],
      [0.25, 0.15],
      [0.75, 0.85],
      [1, 1],
    ];
    const f = curve(pts);
    for (const [x, y] of pts) expect(f(x)).toBeCloseTo(y, 5);
    let last = -1;
    for (let i = 0; i <= 200; i++) {
      const y = f(i / 200);
      expect(y).toBeGreaterThanOrEqual(last - 1e-6);
      last = y;
    }
  });

  it("sorts and clamps", () => {
    expect(
      normalise([
        [0.8, 1.2],
        [0.2, -0.1],
      ]),
    ).toEqual([
      [0.2, 0],
      [0.8, 1],
    ]);
  });

  it("dragging a point off the graph removes only that point, and back restores it", () => {
    const start: Point[] = [
      [0, 0],
      [0.3, 0.2],
      [0.7, 0.8],
      [1, 1],
    ];
    // Three moves below the graph, then back up (issue #11).
    for (const y of [-0.2, -0.3, -0.4])
      expect(dragPoint(start, 1, [0.3, y])).toEqual([
        [0, 0],
        [0.7, 0.8],
        [1, 1],
      ]);
    expect(dragPoint(start, 1, [0.3, 0.1])).toEqual([
      [0, 0],
      [0.3, 0.1],
      [0.7, 0.8],
      [1, 1],
    ]);
    expect(start[1]).toEqual([0.3, 0.2]); // not mutated
  });

  it("end points are never removed, and points stay between their neighbours", () => {
    const start: Point[] = [
      [0, 0],
      [0.5, 0.5],
      [1, 1],
    ];
    expect(dragPoint(start, 2, [0.8, 1.5])).toEqual([
      [0, 0],
      [0.5, 0.5],
      [0.8, 1],
    ]);
    expect(dragPoint(start, 1, [0.995, 0.4])[1]).toEqual([0.99, 0.4]);
  });
});
