import { describe, expect, test } from "bun:test";
import { defaultAdjustments } from "./adjustments";
import { dragged, fits, flipped, frameSize, heldAspect, largest, quarterTurn, Rect, shapeAspect, shrinkToFit, straightened, toCrop, toFrame, toPhoto, toRect, withAspect } from "./crop";

const photo = { width: 600, height: 400 };
const whole: Rect = { x: 300, y: 200, width: 600, height: 400 };

/** The four corners of a tilted crop, on the photo. */
function corners(rect: Rect, angle: number): [number, number][] {
  return ([[-1, -1], [1, -1], [1, 1], [-1, 1]] as const).map(([sx, sy]) => {
    const [dx, dy] = toPhoto([(sx * rect.width) / 2, (sy * rect.height) / 2], angle);
    return [rect.x + dx, rect.y + dy];
  });
}
const onPhoto = (rect: Rect, angle: number) =>
  corners(rect, angle).every(([x, y]) => x > -0.02 && x < photo.width + 0.02 && y > -0.02 && y < photo.height + 0.02);

describe("fitting a crop on the photo", () => {
  test("the whole photo fits only when it is level", () => {
    expect(fits(whole, 0, photo)).toBe(true);
    expect(fits(whole, 3, photo)).toBe(false);
  });

  test("`fits` agrees with where the corners actually are", () => {
    for (const angle of [-40, -7, 0, 12, 45]) {
      for (const rect of [whole, { x: 200, y: 150, width: 300, height: 200 }, { x: 520, y: 330, width: 150, height: 130 }]) {
        expect(fits(rect, angle, photo)).toBe(onPhoto(rect, angle));
      }
    }
  });

  test("the largest crop keeps its shape and touches the photo's edge", () => {
    for (const angle of [0, 5, -20, 45]) {
      const rect = largest(1.5, angle, photo);
      expect(rect.width / rect.height).toBeCloseTo(1.5);
      expect(fits(rect, angle, photo)).toBe(true);
      expect(fits({ ...rect, width: rect.width * 1.01, height: rect.height * 1.01 }, angle, photo)).toBe(false);
    }
    expect(largest(1.5, 0, photo)).toEqual(whole);
  });

  test("shrinking keeps the centre and the shape", () => {
    const off: Rect = { x: 450, y: 250, width: 400, height: 200 };
    const fitted = shrinkToFit(off, 10, photo);
    expect([fitted.x, fitted.y]).toEqual([450, 250]);
    expect(fitted.width / fitted.height).toBeCloseTo(2);
    expect(fits(fitted, 10, photo)).toBe(true);
  });

  test("choosing a shape gives the biggest crop of that shape around the same centre", () => {
    const square = withAspect(whole, 1, 0, photo);
    expect(square).toEqual({ x: 300, y: 200, width: 400, height: 400 });
    // And back to a wide shape grows it again instead of shrinking further.
    expect(withAspect(square, 1.5, 0, photo)).toEqual(whole);
  });
});

describe("straightening", () => {
  test("an untouched crop shrinks as the photo tilts and grows back when levelled", () => {
    const tilted = straightened(whole, 0, 8, photo);
    expect(tilted.width).toBeLessThan(600);
    expect(tilted.width / tilted.height).toBeCloseTo(1.5);
    expect(fits(tilted, 8, photo)).toBe(true);
    const level = straightened(tilted, 8, 0, photo);
    expect(level.width).toBeCloseTo(600);
    expect(level.height).toBeCloseTo(400);
  });

  test("a crop you placed yourself only shrinks if it has to", () => {
    const placed: Rect = { x: 200, y: 200, width: 200, height: 200 };
    expect(straightened(placed, 0, 5, photo)).toEqual(placed);
    const near: Rect = { x: 110, y: 200, width: 200, height: 200 };
    const squeezed = straightened(near, 0, 30, photo);
    expect(squeezed.width).toBeLessThan(200);
    expect(fits(squeezed, 30, photo)).toBe(true);
  });

  test("the uncropped view is big enough for the whole tilted photo", () => {
    const tilted = { ...defaultAdjustments(), straighten: 30 };
    const box = frameSize(photo, tilted, true);
    expect(box.width).toBeCloseTo(600 * Math.cos(Math.PI / 6) + 400 * 0.5);
    expect(frameSize(photo, { ...tilted, straighten: 0, rotation: 1 }, true)).toEqual({ width: 400, height: 600 });
  });

  test("screen and photo directions convert back and forth", () => {
    const [x, y] = toFrame(toPhoto([30, -12], 17), 17);
    expect(x).toBeCloseTo(30);
    expect(y).toBeCloseTo(-12);
    // Tilted clockwise, what is "right" on screen points right and up along the photo.
    const [px, py] = toPhoto([1, 0], 20);
    expect(px).toBeGreaterThan(0);
    expect(py).toBeLessThan(0);
  });
});

describe("dragging the frame", () => {
  const start: Rect = { x: 300, y: 200, width: 200, height: 100 };

  test("a corner moves while the opposite corner stays", () => {
    const next = dragged(start, { x: 1, y: 1 }, [40, 20], 0, photo, null);
    expect(next).toEqual({ x: 320, y: 210, width: 240, height: 120 });
  });

  test("an edge changes one dimension only", () => {
    const next = dragged(start, { x: -1, y: 0 }, [-50, 30], 0, photo, null);
    expect(next).toEqual({ x: 275, y: 200, width: 250, height: 100 });
  });

  test("dragging stops at the photo's edge", () => {
    const next = dragged(start, { x: 1, y: 0 }, [5000, 0], 0, photo, null);
    expect(next.x + next.width / 2).toBeCloseTo(600, 0);
    expect(fits(next, 0, photo)).toBe(true);
  });

  test("moving slides along an edge instead of sticking", () => {
    const next = dragged(start, { x: 0, y: 0 }, [5000, 60], 0, photo, null);
    expect(next.x).toBeCloseTo(500, 0);
    expect(next.y).toBeCloseTo(260, 0);
    expect([next.width, next.height]).toEqual([200, 100]);
  });

  test("a locked shape is kept, from corners and from edges", () => {
    const corner = dragged(start, { x: 1, y: 1 }, [60, 0], 0, photo, 2);
    expect(corner.width / corner.height).toBeCloseTo(2);
    expect(corner.width).toBeCloseTo(260);
    // The top-left corner did not move.
    expect(corner.x - corner.width / 2).toBeCloseTo(200);
    expect(corner.y - corner.height / 2).toBeCloseTo(150);

    const edge = dragged(start, { x: 0, y: 1 }, [0, 30], 0, photo, 2);
    expect(edge.width / edge.height).toBeCloseTo(2);
    expect(edge.height).toBeCloseTo(130);
    expect(edge.x).toBeCloseTo(300);
  });

  test("a frame can't be dragged down to nothing", () => {
    const next = dragged(start, { x: 1, y: 1 }, [-5000, -5000], 0, photo, null);
    expect(next.width).toBeGreaterThan(10);
    expect(next.height).toBeGreaterThan(10);
  });

  test("on a tilted photo every drag still ends on the photo", () => {
    const angle = 15;
    const tilted = largest(1.5, angle, photo);
    for (const handle of [{ x: 1, y: 1 }, { x: -1, y: 0 }, { x: 0, y: -1 }, { x: 0, y: 0 }] as const) {
      for (const delta of [[300, 200], [-300, 50], [40, -400]] as [number, number][]) {
        const next = dragged({ ...tilted, width: tilted.width / 2, height: tilted.height / 2 }, handle, delta, angle, photo, null);
        expect(onPhoto(next, angle)).toBe(true);
      }
    }
  });
});

describe("choosing a shape", () => {
  const tall: Rect = { x: 300, y: 200, width: 200, height: 300 };
  const square: Rect = { x: 300, y: 200, width: 300, height: 300 };

  test("keeps the crop the way up it already is", () => {
    expect(shapeAspect(3 / 2, whole, photo)).toBeCloseTo(3 / 2);
    expect(shapeAspect(3 / 2, tall, photo)).toBeCloseTo(2 / 3);
  });

  test("turns the photo's own shape to match a tall crop", () => {
    // "Original" on a wide photo, with a tall crop, is the photo's shape stood up.
    expect(shapeAspect(photo.width / photo.height, tall, photo)).toBeCloseTo(2 / 3);
    // And on a tall photo, a wide crop gets the photo's shape laid down.
    const portrait = { width: 400, height: 600 };
    expect(shapeAspect(portrait.width / portrait.height, { x: 200, y: 300, width: 300, height: 200 }, portrait)).toBeCloseTo(3 / 2);
  });

  test("a square crop follows the photo, however it rounds", () => {
    const portrait = { width: 400, height: 600 };
    for (const nudge of [-0.01, 0, 0.01]) {
      const rect = { ...square, height: square.height + nudge };
      expect(shapeAspect(3 / 2, rect, photo)).toBeCloseTo(3 / 2);
      expect(shapeAspect(3 / 2, rect, portrait)).toBeCloseTo(2 / 3);
    }
  });
});

describe("turning and flipping", () => {
  const base = { ...defaultAdjustments(), straighten: 6, crop: { x: 0.25, y: 0.4, width: 0.3, height: 0.2 } };

  test("four quarter-turns come back to the start", () => {
    let a = base;
    for (let i = 0; i < 4; i++) a = { ...a, ...quarterTurn(a, true) };
    expect(a.rotation).toBe(0);
    expect(a.crop.x).toBeCloseTo(0.25);
    expect(a.crop.y).toBeCloseTo(0.4);
    expect(a.crop.width).toBeCloseTo(0.3);
  });

  test("a turn carries the crop round with the picture", () => {
    const turned = quarterTurn(base, true);
    // What was near the left edge is now near the top.
    expect(turned.crop).toEqual({ x: 1 - 0.4, y: 0.25, width: 0.2, height: 0.3 });
    expect(quarterTurn({ ...base, ...turned }, false).crop.x).toBeCloseTo(0.25);
  });

  test("a turn turns a mirror too", () => {
    const mirrored = { ...base, flipHorizontal: true };
    const turned = quarterTurn(mirrored, true);
    expect([turned.flipHorizontal, turned.flipVertical]).toEqual([false, true]);
  });

  test("a flip mirrors the crop and reverses the tilt", () => {
    const flippedAcross = flipped(base, true);
    expect(flippedAcross.crop.x).toBeCloseTo(0.75);
    expect(flippedAcross.straighten).toBe(-6);
    expect(flippedAcross.flipHorizontal).toBe(true);
    const back = flipped({ ...base, ...flippedAcross }, true);
    expect(back.crop.x).toBeCloseTo(0.25);
    expect(back.straighten).toBe(6);
  });
});

describe("a chosen shape holds only while the crop has it", () => {
  /** The crop as it comes back from the library: through 0..1 and single precision. */
  const stored = (rect: Rect): Rect => {
    const crop = toCrop(rect, photo);
    const f32 = (n: number) => Math.fround(n);
    return toRect({ x: f32(crop.x), y: f32(crop.y), width: f32(crop.width), height: f32(crop.height) }, photo);
  };

  test("a 3:2 crop doesn't hold a square shape, but holds 3:2", () => {
    expect(heldAspect(whole, 1)).toBeNull();
    expect(heldAspect(whole, 1.5)).toBe(1.5);
    expect(heldAspect(whole, null)).toBeNull();
  });

  test("a crop made to a shape still holds it once stored", () => {
    const square = withAspect(whole, 1, 7, photo);
    expect(heldAspect(stored(square), 1)).toBe(1);
    const resized = dragged({ x: 300, y: 200, width: 400, height: 300 }, { x: 1, y: 1 }, [-37, 11], 0, photo, 4 / 3);
    expect(heldAspect(stored(resized), 4 / 3)).toBe(4 / 3);
    const tilted = straightened(withAspect(whole, 16 / 9, 0, photo), 0, -12, photo);
    expect(heldAspect(stored(tilted), 16 / 9)).toBe(16 / 9);
  });

  test("a shape holds the way up the crop is now", () => {
    // Undoing a swap to portrait leaves a tall crop with the wide shape chosen.
    const tall = withAspect(whole, 2 / 3, 0, photo);
    expect(heldAspect(tall, 3 / 2)).toBe(2 / 3);
    expect(heldAspect(tall, 2 / 3)).toBe(2 / 3);
    expect(heldAspect(tall, 4 / 3)).toBeNull();
  });
});
