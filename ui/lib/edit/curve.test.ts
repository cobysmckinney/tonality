import { describe, expect, it } from "vitest";
import { curve, isIdentity, normalise } from "./curve";

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
});
