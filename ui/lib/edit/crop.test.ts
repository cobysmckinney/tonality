import { describe, expect, it } from "vitest";
import type { Crop } from "$lib/bindings/Crop";
import { FULL, fits, flip, levelAngle, shrinkToFit, towards, turn, withAspect, type Rect } from "./crop";

const crop = (p: Partial<Crop> = {}): Crop => ({ turns: 0, flip: false, angle: 0, rect: [0, 0, 1, 1], ...p });

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
