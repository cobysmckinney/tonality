import { describe, expect, test } from "bun:test";
import { Adjustments, defaultAdjustments, Point } from "./adjustments";
import { apply, frameToSource, newMask, newShape, screenMap } from "./masks";

const photo = { width: 300, height: 200 };
const edited = (change: Partial<Adjustments>): Adjustments => ({ ...defaultAdjustments(), ...change });
const near = (a: Point, b: Point) => {
  expect(a[0]).toBeCloseTo(b[0], 6);
  expect(a[1]).toBeCloseTo(b[1], 6);
};

describe("from the picture to the photo file", () => {
  test("an untouched photo is the file itself", () => {
    near(apply(frameToSource(photo, defaultAdjustments()), [0.25, 0.75]), [0.25, 0.75]);
  });

  test("turned a quarter clockwise, the left of the file is at the top", () => {
    // As the GPU draws it: see masks_stay_on_the_photo_when_it_is_turned in tests/masks.rs.
    near(apply(frameToSource(photo, edited({ rotation: 1 })), [0.5, 0.25]), [0.25, 0.5]);
  });

  test("a crop picks out part of the file", () => {
    const cropped = edited({ crop: { x: 0.75, y: 0.5, width: 0.5, height: 0.5 } });
    near(apply(frameToSource(photo, cropped), [0, 0]), [0.5, 0.25]);
    near(apply(frameToSource(photo, cropped), [1, 1]), [1, 0.75]);
  });
});

describe("on screen", () => {
  const view = (adjustments: Adjustments) => ({
    photo,
    adjustments,
    region: { x: 0.25, y: 0.25, width: 0.5, height: 0.5 },
    // The same shape as the half of the picture it shows.
    shown: { width: 400, height: (400 * 100) / 150 },
  });

  test("positions come back to where they started, however the photo is framed", () => {
    const framed = edited({ rotation: 3, flipHorizontal: true, straighten: 7, crop: { x: 0.4, y: 0.6, width: 0.5, height: 0.6 } });
    const map = screenMap(view(framed));
    for (const point of [
      [0.1, 0.2],
      [0.5, 0.5],
      [0.9, 0.7],
    ] as Point[]) {
      near(map.toPhoto(map.toScreen(point)), point);
    }
  });

  test("lengths scale with the zoom, the same both ways", () => {
    // Half of a 300-pixel-wide picture across 400 screen pixels.
    const map = screenMap(view(defaultAdjustments()));
    expect(map.scale).toBeCloseTo((400 / 150) * 300, 6);
    const [dx, dy] = map.direction([0, 10]);
    expect(Math.hypot(dx, dy)).toBeCloseTo((400 / 150) * 10, 6);
  });
});

describe("new masks", () => {
  test("shapes start in the middle of the cropped picture", () => {
    const cropped = edited({ crop: { x: 0.25, y: 0.5, width: 0.5, height: 1 } });
    const radial = newShape("radial", photo, cropped);
    if (radial.kind !== "radial") throw new Error("expected a radial");
    near(radial.center, [0.25, 0.5]);
    // A third of the crop's shorter side (150 pixels), in shares of the 300-pixel long side.
    expect(radial.radius[0]).toBeCloseTo(0.165, 5);
    const linear = newShape("linear", photo, edited({ rotation: 1 }));
    if (linear.kind !== "linear") throw new Error("expected a linear");
    // Falling down the screen is falling across the file from the left when it is turned.
    expect(linear.from[0]).toBeLessThan(linear.to[0]);
  });

  test("ids and names are never reused", () => {
    const first = newMask([], "brush", photo, defaultAdjustments());
    const second = newMask([first, { ...first, id: 5, name: "Mask 2" }], "radial", photo, defaultAdjustments());
    expect([first.id, first.name]).toEqual([1, "Mask 1"]);
    expect([second.id, second.name]).toEqual([6, "Mask 3"]);
  });
});
