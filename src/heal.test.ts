import { describe, expect, test } from "bun:test";
import { Adjustments, defaultAdjustments, describeChange, Point, Spot } from "./adjustments";
import {
  HEAL_SIZE,
  HEAL_STEPS,
  healRadius,
  MAX_POINTS,
  movePatch,
  moveSpot,
  pathThrough,
  patchPoints,
  simplify,
  sizeForRadius,
  specksAsSpots,
  travel,
} from "./heal";
import { screenMap } from "./masks";
import { PRESET_SECTIONS, settingsFrom } from "./presets";

const photo = { width: 300, height: 200 };
const edited = (change: Partial<Adjustments>): Adjustments => ({ ...defaultAdjustments(), ...change });
const near = (a: Point, b: Point, digits = 6) => {
  expect(a[0]).toBeCloseTo(b[0], digits);
  expect(a[1]).toBeCloseTo(b[1], digits);
};
const spot: Spot = { points: [[0.2, 0.3]], radius: 0.01, source: [0.25, 0.3] };

describe("the size of a spot", () => {
  test("runs from a pixel or so to a large patch, finer at the small end", () => {
    expect(healRadius(1)).toBeLessThan(0.00001);
    expect(healRadius(100)).toBe(0.08);
    // The default is a speck a few dozen pixels across on a 6000-pixel photo.
    expect(healRadius(HEAL_SIZE) * 6000).toBeGreaterThan(10);
    expect(healRadius(HEAL_SIZE) * 6000).toBeLessThan(30);
  });

  test("reads back as the slider value that made it", () => {
    for (const size of [1, 7, HEAL_SIZE, 55, 100]) expect(sizeForRadius(healRadius(size))).toBe(size);
  });
});

describe("a line drawn by hand", () => {
  test("keeps only its ends when it is straight", () => {
    const straight: Point[] = Array.from({ length: 50 }, (_, i) => [i * 2, 10 + i * 0.5]);
    expect(simplify(straight, 1)).toEqual([straight[0], straight[49]]);
  });

  test("keeps its corners", () => {
    const corner: Point[] = [...Array.from({ length: 20 }, (_, i): Point => [i, 0]), ...Array.from({ length: 20 }, (_, i): Point => [19, i + 1])];
    const kept = simplify(corner, 1);
    expect(kept).toEqual([
      [0, 0],
      [19, 0],
      [19, 20],
    ]);
  });

  test("never keeps more points than a spot holds, ends included", () => {
    const wiggle: Point[] = Array.from({ length: 400 }, (_, i) => [i, (i % 2) * 10]);
    const kept = simplify(wiggle, 0.5);
    expect(kept.length).toBe(MAX_POINTS);
    expect(kept[0]).toEqual(wiggle[0]);
    expect(kept[MAX_POINTS - 1]).toEqual(wiggle[399]);
  });

  test("measures how far it runs, so a click is told from a drag", () => {
    expect(travel([[5, 5]])).toBe(0);
    expect(travel([[0, 0], [3, 4], [3, 10]])).toBe(11);
  });

  test("is drawn as a path, a lone point as a dot", () => {
    expect(pathThrough([[1, 2]])).toBe("M1 2L1 2");
    expect(pathThrough([[1, 2], [3, 4]])).toBe("M1 2L3 4");
  });
});

describe("moving spots", () => {
  test("moving a spot leaves its patch where it is", () => {
    const moved = moveSpot(spot, [0.1, -0.05]);
    near(moved.points[0], [0.3, 0.25]);
    expect(moved.source).toEqual(spot.source);
  });

  test("moving the patch leaves the spot where it is", () => {
    const moved = movePatch(spot, [0, 0.1]);
    near(moved.source, [0.25, 0.4]);
    expect(moved.points).toEqual(spot.points);
  });

  test("a line's patch is the line moved to its source", () => {
    const line: Spot = { points: [[0.1, 0.1], [0.4, 0.2]], radius: 0.01, source: [0.1, 0.15] };
    const patch = patchPoints(line);
    near(patch[0], [0.1, 0.15]);
    near(patch[1], [0.4, 0.25]);
  });

  test("specks found as dust become spots over them, with the patch found for each", () => {
    const [made] = specksAsSpots([{ center: [0.5, 0.5], radius: 0.002, source: [0.51, 0.5] }]);
    expect(made).toEqual({ points: [[0.5, 0.5]], radius: 0.002, source: [0.51, 0.5] });
  });
});

describe("spots stay on the photo file", () => {
  const view = (adjustments: Adjustments) =>
    screenMap({ photo, adjustments, region: { x: 0, y: 0, width: 1, height: 1 }, shown: { width: 600, height: 400 } });

  test("a spot placed on screen is on the same place of the file however the photo is framed", () => {
    const framings = [
      edited({}),
      edited({ rotation: 1 }),
      edited({ rotation: 2, flipHorizontal: true }),
      edited({ rotation: 3, flipVertical: true, straighten: 7, crop: { x: 0.4, y: 0.6, width: 0.5, height: 0.6 } }),
    ];
    for (const framing of framings) {
      const map = view(framing);
      const placed = map.toPhoto([120, 90]);
      near(map.toScreen(placed), [120, 90], 4);
    }
  });

  test("after a quarter-turn the same speck is somewhere else on screen", () => {
    const speck: Point = [0.25, 0.5];
    const upright = view(edited({}));
    const turned = screenMap({
      photo,
      adjustments: edited({ rotation: 1 }),
      region: { x: 0, y: 0, width: 1, height: 1 },
      shown: { width: 400, height: 600 },
    });
    near(upright.toScreen(speck), [150, 200], 4);
    // Turned clockwise, the left of the file is at the top.
    near(turned.toScreen(speck), [200, 150], 4);
    // And a spot's radius is the same share of the photo's longer side either way.
    expect(upright.scale).toBeCloseTo(turned.scale, 6);
  });
});

describe("the history and presets", () => {
  test("names each kind of change plainly", () => {
    expect(HEAL_STEPS.added(spot)).toBe("Healed a spot");
    expect(HEAL_STEPS.added({ ...spot, points: [[0.1, 0.1], [0.2, 0.2]] })).toBe("Healed a line");
    expect(HEAL_STEPS.specks(1)).toBe("Removed a speck");
    expect(HEAL_STEPS.specks(23)).toBe("Removed 23 specks");
    expect(describeChange(defaultAdjustments(), edited({ spots: [spot] }))).toBe("Heal");
  });

  test("a preset never holds spots", () => {
    const healed = edited({ spots: [spot], exposure: 1 });
    const settings = settingsFrom(healed, PRESET_SECTIONS.map((section) => section.title));
    expect("spots" in settings).toBe(false);
    expect(settings.exposure).toBe(1);
  });
});
