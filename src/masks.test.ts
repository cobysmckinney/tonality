import { describe, expect, test } from "bun:test";
import { Adjustments, defaultAdjustments, Point, Shape } from "./adjustments";
import {
  apply,
  canAdd,
  circleShape,
  foundCount,
  frameToSource,
  fromOriginal,
  matteKey,
  MAX_FOUND,
  newMask,
  newShape,
  onOriginal,
  screenMap,
  stillHeld,
} from "./masks";

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

describe("between the edit and the original", () => {
  test("the middle of a top-left quarter crop is a quarter of the way into the original", () => {
    const quarter = edited({ crop: { x: 0.25, y: 0.25, width: 0.5, height: 0.5 } });
    near(onOriginal(photo, quarter, [0.5, 0.5]), [0.25, 0.25]);
    near(fromOriginal(photo, quarter, [0.25, 0.25]), [0.5, 0.5]);
  });

  test("a turned or flipped photo is unturned and unflipped", () => {
    near(onOriginal(photo, edited({ rotation: 1 }), [0.5, 0.25]), [0.25, 0.5]);
    near(onOriginal(photo, edited({ flipHorizontal: true }), [0.2, 0.3]), [0.8, 0.3]);
  });

  test("there and back is the same point, however the photo is framed", () => {
    const framed = edited({ rotation: 3, flipVertical: true, straighten: -12, crop: { x: 0.45, y: 0.55, width: 0.4, height: 0.5 } });
    for (const point of [
      [0, 0],
      [0.5, 0.5],
      [0.8, 0.3],
    ] as Point[]) {
      near(fromOriginal(photo, framed, onOriginal(photo, framed, point)), point);
    }
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

  test("the subject and the background start from the same found subject", () => {
    expect(newShape("subject", photo, defaultAdjustments())).toEqual({ kind: "subject" });
    const subject = newMask([], "subject", photo, defaultAdjustments());
    const background = newMask([subject], "background", photo, defaultAdjustments());
    expect([subject.name, subject.invert]).toEqual(["Subject", false]);
    expect([background.name, background.invert]).toEqual(["Background", true]);
    expect(background.parts).toEqual([{ mode: "add", shape: { kind: "subject" } }]);
    expect(newMask([subject, background], "subject", photo, defaultAdjustments()).name).toBe("Subject 2");
  });

  test("a circle is kept short, and found parts have a limit of their own", () => {
    const loop = Array.from({ length: 500 }, (_, i) => [0.5 + 0.2 * Math.cos(i / 80), 0.5 + 0.2 * Math.sin(i / 80)] as Point);
    const shape = circleShape(loop);
    expect(shape.kind).toBe("object");
    expect(shape.kind === "object" && shape.points.length).toBeLessThanOrEqual(160);
    expect(newMask([], "sky", photo, defaultAdjustments()).name).toBe("Sky");

    const objects = Array.from({ length: MAX_FOUND }, (_, i) =>
      newMask([], "object", photo, defaultAdjustments(), { kind: "object", points: [[i / 10, 0], [0.5, 0.5], [0, 0.5]] }),
    ).map((mask, i) => ({ ...mask, id: i + 1 }));
    expect(canAdd(objects, "object")).toBe(false);
    expect(canAdd(objects, "linear")).toBe(true);
    // The subject twice counts once.
    const subjects = [newMask([], "subject", photo, defaultAdjustments()), newMask([], "background", photo, defaultAdjustments())];
    expect(foundCount(subjects)).toBe(1);
    expect(canAdd(subjects, "subject")).toBe(true);
  });
});

describe("a drag on the photo", () => {
  const start = { maskId: 1, partIndex: 0 };

  test("keeps hold while the same mask and part are chosen", () => {
    expect(stillHeld(start, { maskId: 1, partIndex: 0, circling: false })).toBe(true);
  });

  test("lets go when the mask is let go of or another is chosen", () => {
    expect(stillHeld(start, { maskId: null, partIndex: null, circling: false })).toBe(false);
    expect(stillHeld(start, { maskId: 2, partIndex: 0, circling: false })).toBe(false);
  });

  test("lets go when another part is chosen or a loop is drawn", () => {
    expect(stillHeld(start, { maskId: 1, partIndex: 1, circling: false })).toBe(false);
    expect(stillHeld(start, { maskId: 1, partIndex: 0, circling: true })).toBe(false);
  });
});

describe("the masks' thumbnails", () => {
  const radial = newMask([], "radial", photo, defaultAdjustments());
  const masked = edited({ masks: [radial] });

  test("are left alone by sliders that can't change a mask", () => {
    const key = matteKey(masked);
    expect(matteKey({ ...masked, exposure: 1, saturation: 30, grain: 20 })).toBe(key);
    const brighter = { ...radial, name: "Face", adjustments: { ...radial.adjustments, exposure: 0.5 } };
    expect(matteKey({ ...masked, masks: [brighter] })).toBe(key);
  });

  test("are redrawn when a part, the way a mask combines, or the framing changes", () => {
    const key = matteKey(masked);
    const moved = { ...radial, parts: [{ ...radial.parts[0], shape: { ...radial.parts[0].shape, angle: 30 } as Shape }] };
    expect(matteKey({ ...masked, masks: [moved] })).not.toBe(key);
    expect(matteKey({ ...masked, masks: [{ ...radial, invert: true }] })).not.toBe(key);
    expect(matteKey({ ...masked, rotation: 1 })).not.toBe(key);
    expect(matteKey({ ...masked, crop: { x: 0.5, y: 0.5, width: 0.5, height: 0.5 } })).not.toBe(key);
  });

  test("follow the exposure only when a brightness range reads it", () => {
    const ranged = edited({ masks: [newMask([], "luminance", photo, defaultAdjustments())] });
    expect(matteKey({ ...ranged, exposure: 1 })).not.toBe(matteKey(ranged));
  });
});
