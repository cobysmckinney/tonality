import { expect, test } from "bun:test";
import { Adjustments, defaultAdjustments, Point } from "./adjustments";
import { pickedPoint } from "./film";
import { apply, frameToSource, invert, uncroppedToSource } from "./masks";

const photo = { width: 300, height: 200 };
const edited = (change: Partial<Adjustments>): Adjustments => ({ ...defaultAdjustments(), ...change });
const near = (a: Point, b: Point) => {
  expect(a[0]).toBeCloseTo(b[0], 6);
  expect(a[1]).toBeCloseTo(b[1], 6);
};
const full = { x: 0, y: 0, width: 1, height: 1 };

test("a click on the uncropped photo lands where it is on the file, whatever the crop", () => {
  const cropped = edited({ crop: { x: 0.3, y: 0.6, width: 0.4, height: 0.5 } });
  const shown = { width: 600, height: 400 };
  // The whole photo is shown, so the crop doesn't matter.
  near(pickedPoint(photo, cropped, full, shown, [0, 0]), [0, 0]);
  near(pickedPoint(photo, cropped, full, shown, [150, 300]), [0.25, 0.75]);
  // A quarter-turn: the top left on screen was the bottom left of the file.
  const turned = edited({ rotation: 1 });
  near(pickedPoint(photo, turned, full, { width: 200, height: 300 }, [0, 0]), [0, 1]);
  // Zoomed in on the right half of the picture.
  near(pickedPoint(photo, cropped, { x: 0.5, y: 0, width: 0.5, height: 1 }, shown, [0, 0]), [0.5, 0]);
});

test("the uncropped frame holds the whole tilted photo", () => {
  const tilted = edited({ straighten: 10 });
  const map = uncroppedToSource(photo, tilted);
  // Its centre is the photo's, and the photo's corners are inside it.
  near(apply(map, [0.5, 0.5]), [0.5, 0.5]);
  for (const corner of [[0, 0], [1, 0], [0, 1], [1, 1]] as Point[]) {
    const [x, y] = apply(invert(map), corner);
    expect(Math.min(x, y)).toBeGreaterThanOrEqual(-1e-9);
    expect(Math.max(x, y)).toBeLessThanOrEqual(1 + 1e-9);
  }
  // Without a tilt or a crop, it is the cropped frame.
  expect(uncroppedToSource(photo, defaultAdjustments())).toEqual(frameToSource(photo, defaultAdjustments()));
});

test("a click on the cropped photo lands inside the crop", () => {
  // The white balance picker works on the photo as it is normally shown.
  const cropped = edited({ crop: { x: 0.3, y: 0.6, width: 0.4, height: 0.5 } });
  const shown = { width: 240, height: 200 };
  near(pickedPoint(photo, cropped, full, shown, [0, 0], false), [0.1, 0.35]);
  near(pickedPoint(photo, cropped, full, shown, [240, 200], false), [0.5, 0.85]);
});
