import { describe, expect, it } from "vitest";
import { centerAt, grabOffset, inVisible, overviewSize, visibleRect } from "./navigator";

describe("overviewSize", () => {
  it("keeps the frame's aspect with the long edge at `edge`", () => {
    expect(overviewSize(6000, 4000, 150)).toEqual({ w: 150, h: 100 });
    expect(overviewSize(4000, 6000, 150)).toEqual({ w: 100, h: 150 });
  });
  it("never goes below a pixel", () => {
    expect(overviewSize(10000, 1, 100)).toEqual({ w: 100, h: 1 });
  });
});

describe("visibleRect", () => {
  it("is the whole frame when it fits inside the pane", () => {
    expect(visibleRect({ x: 50, y: 20, w: 400, h: 300 }, 500, 340)).toEqual({
      x: 0,
      y: 0,
      w: 1,
      h: 1,
    });
  });
  it("is the middle quarter at 2× around the centre", () => {
    // A 1000×800 pane over a 4000×3200 frame centred on it.
    const r = visibleRect({ x: -1500, y: -1200, w: 4000, h: 3200 }, 1000, 800);
    expect(r.x).toBeCloseTo(0.375);
    expect(r.y).toBeCloseTo(0.375);
    expect(r.w).toBeCloseTo(0.25);
    expect(r.h).toBeCloseTo(0.25);
  });
  it("is clipped to the frame where the pane shows past its edge", () => {
    // Zoomed in on width only: the frame is narrower than tall pane.
    const r = visibleRect({ x: -500, y: 100, w: 2000, h: 600 }, 1000, 800);
    expect(r).toEqual({ x: 0.25, y: 0, w: 0.5, h: 1 });
  });
});

describe("pressing on the overview", () => {
  const size = { w: 200, h: 100 };
  const vis = { x: 0.25, y: 0.25, w: 0.25, h: 0.5 };

  it("knows when a press lands on the rectangle", () => {
    expect(inVisible(60, 40, size, vis)).toBe(true);
    expect(inVisible(20, 40, size, vis)).toBe(false);
  });

  it("brings the rectangle's centre to a press outside it", () => {
    const off = grabOffset(160, 50, size, vis);
    expect(off).toEqual({ x: 0, y: 0 });
    expect(centerAt(160, 50, size, vis, off)).toEqual({ x: 0.8, y: 0.5 });
  });

  it("drags the rectangle from where it was grabbed, without a jump", () => {
    // Grab near its left edge: the centre stays put until the pointer moves.
    const off = grabOffset(52, 50, size, vis);
    const c0 = centerAt(52, 50, size, vis, off);
    expect(c0.x).toBeCloseTo(0.375);
    expect(c0.y).toBeCloseTo(0.5);
    const c1 = centerAt(72, 50, size, vis, off);
    expect(c1.x).toBeCloseTo(0.475);
  });

  it("keeps the rectangle inside the frame", () => {
    expect(centerAt(0, 0, size, vis)).toEqual({ x: 0.125, y: 0.25 });
    expect(centerAt(200, 100, size, vis)).toEqual({ x: 0.875, y: 0.75 });
  });

  it("centres an axis the view already shows whole", () => {
    expect(centerAt(10, 90, size, { x: 0.4, y: 0, w: 0.2, h: 1 }).y).toBe(0.5);
  });
});
