import { describe, expect, test } from "bun:test";
import { FIT, heldFrame, MAX_ZOOM, pannedView, place, Placement, zoomedView } from "./zoom";

const photo = { width: 4000, height: 3000 };
const stage = { left: 100, top: 50, width: 1000, height: 800 };

/** The canvas as the stage lays it out: centred, at its size in CSS pixels. */
const canvasOf = (placed: Placement, ratio: number) => {
  const [width, height] = [placed.width / ratio, placed.height / ratio];
  return { left: stage.left + (stage.width - width) / 2, top: stage.top + (stage.height - height) / 2, width, height };
};
/** The point of the picture under a point on screen. */
const under = (placed: Placement, ratio: number, x: number, y: number) => {
  const canvas = canvasOf(placed, ratio);
  return [
    placed.region.x + ((x - canvas.left) / canvas.width) * placed.region.width,
    placed.region.y + ((y - canvas.top) / canvas.height) * placed.region.height,
  ];
};

describe("placing the picture on the stage", () => {
  test("fitted, it fills the stage one way and shows all of it", () => {
    const placed = place({ width: 1000, height: 800 }, photo, null, 0.5, 0.5);
    expect(placed.fit).toBe(0.25);
    expect(placed.zoom).toBe(0.25);
    expect([placed.width, placed.height]).toEqual([1000, 750]);
    expect(placed.region).toEqual({ x: 0, y: 0, width: 1, height: 1 });
  });

  test("a picture smaller than the stage isn't blown up to fit", () => {
    const placed = place({ width: 1000, height: 800 }, { width: 600, height: 400 }, null, 0.5, 0.5);
    expect(placed.fit).toBe(1);
    expect([placed.width, placed.height]).toEqual([600, 400]);
  });

  test("zoomed in, it shows the part of the picture the stage has room for", () => {
    const placed = place({ width: 1000, height: 800 }, photo, 1, 0.5, 0.5);
    expect(placed.zoom).toBe(1);
    expect([placed.width, placed.height]).toEqual([1000, 800]);
    expect(placed.region.width).toBeCloseTo(0.25, 9);
    expect(placed.region.height).toBeCloseTo(800 / 3000, 9);
    expect(placed.region.x + placed.region.width / 2).toBeCloseTo(0.5, 9);
  });

  test("the zoom stays between the fit and the most it goes to", () => {
    expect(place({ width: 1000, height: 800 }, photo, 50, 0.5, 0.5).zoom).toBe(MAX_ZOOM);
    expect(place({ width: 1000, height: 800 }, photo, 0.01, 0.5, 0.5).zoom).toBe(0.25);
  });

  test("it can't be moved past its edges", () => {
    const corner = place({ width: 1000, height: 800 }, photo, 1, 0, 1);
    expect(corner.region.x).toBe(0);
    expect(corner.region.y + corner.region.height).toBeCloseTo(1, 9);
  });

  test("zoomed in on a long, thin picture, the short side still fits", () => {
    const placed = place({ width: 1000, height: 800 }, { width: 8000, height: 400 }, 1, 0.5, 0.5);
    expect([placed.width, placed.height]).toEqual([1000, 400]);
    expect(placed.region.height).toBe(1);
    expect(placed.region.y).toBe(0);
  });
});

describe("zooming", () => {
  for (const ratio of [1, 2]) {
    test(`the point under the pointer stays where it is (${ratio} device pixels a pixel)`, () => {
      const available = { width: stage.width * ratio, height: stage.height * ratio };
      let placed = place(available, photo, null, 0.5, 0.5);
      for (const [zoom, x, y] of [
        [placed.fit * 2, 400, 300],
        [1, 900, 700],
        [2.5, 150, 120],
        [1.5, 600, 500],
      ]) {
        const before = under(placed, ratio, x, y);
        const view = zoomedView(placed, photo, ratio, canvasOf(placed, ratio), stage, zoom, x, y);
        expect(view.zoom).toBe(zoom);
        placed = place(available, photo, view.zoom, view.x, view.y);
        const after = under(placed, ratio, x, y);
        expect(after[0]).toBeCloseTo(before[0], 9);
        expect(after[1]).toBeCloseTo(before[1], 9);
      }
    });
  }

  test("zooming out to the fit, or past it, fits the picture again", () => {
    const available = { width: stage.width, height: stage.height };
    const placed = place(available, photo, 1, 0.3, 0.6);
    const canvas = canvasOf(placed, 1);
    expect(zoomedView(placed, photo, 1, canvas, stage, null, 500, 400)).toBe(FIT);
    expect(zoomedView(placed, photo, 1, canvas, stage, placed.fit, 500, 400)).toBe(FIT);
    expect(zoomedView(placed, photo, 1, canvas, stage, 0.1, 500, 400)).toBe(FIT);
    expect(zoomedView(placed, photo, 1, canvas, stage, 9, 500, 400).zoom).toBe(MAX_ZOOM);
  });

  test("a pointer off the picture zooms on its nearest edge", () => {
    const available = { width: stage.width, height: stage.height };
    const placed = place(available, photo, null, 0.5, 0.5);
    // The fitted picture leaves a band above it; a pointer there is over its top edge.
    const view = zoomedView(placed, photo, 1, canvasOf(placed, 1), stage, 1, stage.left + 500, stage.top + 5);
    const zoomed = place(available, photo, view.zoom, view.x, view.y);
    expect(zoomed.region.y).toBe(0);
  });
});

describe("panning", () => {
  const available = { width: stage.width, height: stage.height };

  test("the picture follows the pointer", () => {
    const placed = place(available, photo, 1, 0.5, 0.5);
    const view = pannedView(placed, photo, 1, 100, -40);
    expect(view.zoom).toBe(1);
    expect(view.x).toBeCloseTo(0.5 - 100 / 4000, 9);
    expect(view.y).toBeCloseTo(0.5 + 40 / 3000, 9);
  });

  test("at twice the zoom, or on a sharper screen, a drag moves it less far", () => {
    const placed = place({ width: 2000, height: 1600 }, photo, 2, 0.5, 0.5);
    expect(pannedView(placed, photo, 2, 100, 0).x).toBeCloseTo(0.5 - 100 / 4000, 9);
    const twice = place(available, photo, 2, 0.5, 0.5);
    expect(pannedView(twice, photo, 1, 100, 0).x).toBeCloseTo(0.5 - 50 / 4000, 9);
  });

  test("it stops where its edges reach the stage's", () => {
    const placed = place(available, photo, 1, 0.5, 0.5);
    const view = pannedView(placed, photo, 1, 1e6, -1e6);
    expect(view.x).toBeCloseTo(placed.region.width / 2, 9);
    expect(view.y).toBeCloseTo(1 - placed.region.height / 2, 9);
  });
});

describe("the frame shown while the next one renders", () => {
  test("covers the canvas when it shows the same part", () => {
    const region = { x: 0.2, y: 0.3, width: 0.5, height: 0.4 };
    expect(heldFrame(region, region, { width: 800, height: 600 })).toEqual({ left: 0, top: 0, width: 800, height: 600 });
  });

  test("is scaled and moved to where its part of the picture now sits", () => {
    // Zoomed in twice on the middle: the old frame is twice as big, half of it off each side.
    const held = heldFrame({ x: 0, y: 0, width: 1, height: 1 }, { x: 0.25, y: 0.25, width: 0.5, height: 0.5 }, { width: 800, height: 600 });
    expect(held).toEqual({ left: -400, top: -300, width: 1600, height: 1200 });
    // Panned right by a tenth of the picture: the old frame sits a tenth further left.
    const panned = heldFrame({ x: 0.3, y: 0, width: 0.5, height: 1 }, { x: 0.4, y: 0, width: 0.5, height: 1 }, { width: 500, height: 500 });
    expect(panned.left).toBeCloseTo(-100, 9);
    expect(panned.width).toBe(500);
  });
});
