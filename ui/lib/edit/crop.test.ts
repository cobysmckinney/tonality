import { describe, expect, it } from "vitest";
import type { Crop } from "$lib/bindings/Crop";
import {
  FULL,
  MIN,
  fits,
  flip,
  levelAngle,
  shrinkToFit,
  towards,
  turn,
  withAspect,
  type Rect,
} from "./crop";

const crop = (p: Partial<Crop> = {}): Crop => ({
  turns: 0,
  flip: false,
  angle: 0,
  rect: [0, 0, 1, 1],
  ...p,
});

describe("crop", () => {
  it("full frame fits only when straight", () => {
    expect(fits(FULL, 0, 6000, 4000)).toBe(true);
    expect(fits(FULL, 5, 6000, 4000)).toBe(false);
  });

  it("shrinks to fit when straightened, keeping the centre and shape", () => {
    const r = shrinkToFit(FULL, 10, 6000, 4000);
    expect(fits(r, 10, 6000, 4000)).toBe(true);
    expect(r[0] + r[2] / 2).toBeCloseTo(0.5);
    expect(r[2] / r[3]).toBeCloseTo(1);
    // As large as possible: a little bigger would not fit.
    const k = 1.01;
    const bigger: Rect = [0.5 - (r[2] * k) / 2, 0.5 - (r[3] * k) / 2, r[2] * k, r[3] * k];
    expect(fits(bigger, 10, 6000, 4000)).toBe(false);
  });

  it("a small corner crop straightened off the image moves inwards instead of vanishing", () => {
    // Issue #21: these used to collapse to zero size about their own centre.
    for (const [rect, angle] of [
      [[0, 0, 0.1, 0.1], -10],
      [[0, 0, 0.25, 0.25], -20],
      [[0.9, 0.9, 0.1, 0.1], 15],
      [[0, 0.95, 0.05, 0.05], 45],
    ] as [Rect, number][]) {
      const r = shrinkToFit(rect, angle, 6000, 4000);
      expect(fits(r, angle, 6000, 4000)).toBe(true);
      expect(r[2]).toBeGreaterThanOrEqual(MIN - 1e-9);
      expect(r[3]).toBeGreaterThanOrEqual(MIN - 1e-9);
      expect(r[2] / r[3]).toBeCloseTo(rect[2] / rect[3]);
      expect(r[2]).toBeCloseTo(rect[2]); // small enough to keep its size: it just slides in
      // ...and can still be moved and resized from there.
      const sx = Math.sign(0.5 - r[0] - r[2] / 2),
        sy = Math.sign(0.5 - r[1] - r[3] / 2);
      const moved = towards(r, [r[0] + 0.05 * sx, r[1] + 0.05 * sy, r[2], r[3]], angle, 6000, 4000);
      expect(moved).not.toEqual(r);
      // Grow towards the frame centre (the far side may be against the image edge).
      const grown = towards(
        r,
        [sx > 0 ? r[0] : r[0] - 0.02, sy > 0 ? r[1] : r[1] - 0.02, r[2] + 0.02, r[3] + 0.02],
        angle,
        6000,
        4000,
      );
      expect(grown[2]).toBeGreaterThan(r[2]);
    }
  });

  it("never returns a crop smaller than MIN, even from a degenerate one", () => {
    const r = shrinkToFit([0.05, 0.05, 0, 0], -10, 6000, 4000);
    expect(fits(r, -10, 6000, 4000)).toBe(true);
    expect(Math.min(r[2], r[3])).toBeGreaterThanOrEqual(MIN - 1e-9);
    // A crop that already fits about its centre keeps that centre.
    const c = shrinkToFit([0.4, 0.4, 0.2, 0.2], 5, 6000, 4000);
    expect(c).toEqual([0.4, 0.4, 0.2, 0.2]);
  });

  it("a long thin crop off the image still ends up fitting", () => {
    const r = shrinkToFit([0, 0, 0.02, 1], 40, 6000, 4000);
    expect(fits(r, 40, 6000, 4000)).toBe(true);
    expect(r[2] / r[3]).toBeCloseTo(0.03); // widened to MIN, then kept that shape
  });

  it("moves a crop that starts out of bounds", () => {
    const r = towards([0.05, 0.05, 0, 0], [0.4, 0.4, 0.1, 0.1], -10, 6000, 4000);
    expect(fits(r, -10, 6000, 4000)).toBe(true);
  });

  it("moves only as far as fits", () => {
    const from: Rect = [0.25, 0.25, 0.5, 0.5];
    const r = towards(from, [0.8, 0.25, 0.5, 0.5], 0, 100, 100);
    expect(r[0]).toBeCloseTo(0.5, 3);
  });

  it("aspect ratios are in pixels", () => {
    const r = withAspect(FULL, 1, 0, 6000, 4000);
    expect((r[2] * 6000) / (r[3] * 4000)).toBeCloseTo(1);
    expect(r[3]).toBeCloseTo(1);
  });

  it("turning four times, or flipping twice, is a no-op", () => {
    const c = crop({ rect: [0.1, 0.2, 0.3, 0.4], angle: 3 });
    let t = c;
    for (let i = 0; i < 4; i++) t = turn(t, 1);
    t.rect.forEach((v, i) => expect(v).toBeCloseTo(c.rect[i]));
    expect(turn(turn(c, 1), -1).rect.map((v) => +v.toFixed(6))).toEqual(c.rect);
    const f = flip(flip(c));
    expect(f.flip).toBe(false);
    expect(f.angle).toBe(3);
    f.rect.forEach((v, i) => expect(v).toBeCloseTo(c.rect[i]));
  });

  it("levels a drawn line", () => {
    expect(levelAngle(0, 0, 100, 10)).toBeCloseTo(-5.71, 1);
    expect(levelAngle(0, 0, 10, 100)).toBeCloseTo(5.71, 1); // near-vertical
    expect(levelAngle(0, 0, 2, 2)).toBeNull();
  });
});
