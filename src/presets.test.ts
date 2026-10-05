import { describe, expect, test } from "bun:test";
import { Adjustments, curveSampler, defaultAdjustments, POINT_GAP, same } from "./adjustments";
import { blend, changedSections, holds, isOn, Preset, settingsFrom, withPreset } from "./presets";

const edited = (change: Partial<Adjustments>): Adjustments => ({ ...defaultAdjustments(), ...change });

const fade: Preset = {
  id: -1,
  name: "Fade",
  group: "Color",
  settings: {
    vibrance: 20,
    saturation: -10,
    curves: {
      ...defaultAdjustments().curves,
      master: [
        [0, 0.1],
        [0.5, 0.5],
        [1, 0.9],
      ],
    },
  },
};

describe("what a preset covers", () => {
  test("a new preset covers the sections that were changed", () => {
    expect(changedSections(defaultAdjustments())).toEqual([]);
    const photo = edited({ exposure: 0.5, grain: 20, straighten: 4 });
    expect(changedSections(photo)).toEqual(["Light", "Effects"]);
    // Whole sections are taken, zeros included, and never the framing.
    expect(settingsFrom(photo, ["Effects"])).toEqual({ vignette: 0, grain: 20 });
    expect(Object.keys(settingsFrom(photo, ["Light", "Crop"]))).toEqual([
      "exposure",
      "contrast",
      "highlights",
      "shadows",
      "whites",
      "blacks",
    ]);
  });

  test("applying changes the covered settings and nothing else", () => {
    const photo = edited({ exposure: 0.7, vibrance: 50, crop: { x: 0.4, y: 0.5, width: 0.5, height: 0.5 } });
    const after = withPreset(photo, fade.settings);
    expect(after).toEqual({ ...photo, ...fade.settings });
    expect(after.exposure).toBe(0.7);
    expect(after.crop).toEqual(photo.crop);
  });

  test("a preset is on while its values stand", () => {
    const after = withPreset(edited({ exposure: 1 }), fade.settings);
    expect(isOn(fade, after)).toBe(true);
    expect(isOn(fade, { ...after, grain: 30 })).toBe(true);
    expect(isOn(fade, { ...after, vibrance: 21 })).toBe(false);
    // One that only puts things back to zero would otherwise be on for every unedited photo.
    const reset: Preset = { id: 1, name: "No effects", group: null, settings: { vignette: 0, grain: 0 } };
    expect(isOn(reset, defaultAdjustments())).toBe(false);
    expect(holds(defaultAdjustments(), reset.settings)).toBe(true);
  });
});

describe("comparing settings", () => {
  test("the order fields are written in does not matter", () => {
    expect(same({ hue: 1, saturation: 2, luminance: 3 }, { luminance: 3, hue: 1, saturation: 2 })).toBe(true);
    expect(same({ master: [[0, 0]], red: [] }, { red: [], master: [[0, 0]] })).toBe(true);
    expect(same({ hue: 1 }, { hue: 1, saturation: 0 })).toBe(false);
    expect(same([1, 2], { 0: 1, 1: 2 })).toBe(false);
    expect(same([[0, 0.5]], [[0, 0.51]])).toBe(false);
  });
});

describe("the amount", () => {
  const photo = edited({ vibrance: 40, exposure: 1 });

  test("none of it is the photo as it was, all of it the preset", () => {
    expect(withPreset(photo, fade.settings, 0)).toEqual(photo);
    expect(blend(photo, fade.settings, 1)).toEqual(fade.settings);
  });

  test("sliders move part of the way from where they were", () => {
    const half = withPreset(photo, fade.settings, 0.5);
    expect(half.vibrance).toBe(30);
    expect(half.saturation).toBe(-5);
    expect(half.exposure).toBe(1);
    const double = withPreset(photo, fade.settings, 2);
    expect(double.vibrance).toBe(0);
    expect(double.saturation).toBe(-20);
  });

  test("sliders stay inside their range and on their steps", () => {
    const strong = { grain: 80, exposure: 4, contrast: 33 };
    const doubled = withPreset(defaultAdjustments(), strong, 2);
    expect(doubled.grain).toBe(100);
    expect(doubled.exposure).toBe(5);
    expect(withPreset(defaultAdjustments(), strong, 0.5).contrast).toBe(17);
    expect(withPreset(defaultAdjustments(), { exposure: 0.25 }, 0.5).exposure).toBeCloseTo(0.13, 5);
    expect(withPreset(edited({ grain: 50 }), { grain: 10 }, 2).grain).toBe(0);
  });

  test("curves are mixed as lines", () => {
    const bent = edited({
      curves: {
        ...defaultAdjustments().curves,
        master: [
          [0, 0],
          [0.25, 0.35],
          [1, 1],
        ],
      },
    });
    const half = withPreset(bent, fade.settings, 0.5).curves;
    // A point wherever either curve has one.
    expect(half.master.map((point) => point[0])).toEqual([0, 0.25, 0.5, 1]);
    const [before, preset, mixed] = [bent.curves.master, fade.settings.curves!.master, half.master].map(curveSampler);
    for (const x of [0, 0.25, 0.5, 1]) expect(mixed(x)).toBeCloseTo((before(x) + preset(x)) / 2, 3);
    // A channel neither side bends stays straight.
    expect(half.red).toEqual([
      [0, 0],
      [1, 1],
    ]);
    // Past the preset the fade deepens, but never leaves the box.
    const double = withPreset(defaultAdjustments(), fade.settings, 2).curves.master;
    expect(double[0]).toEqual([0, 0.2]);
    expect(double[2]).toEqual([1, 0.8]);
    const extreme = withPreset(defaultAdjustments(), { curves: { ...bent.curves, master: [[0, 0.7], [1, 1]] } }, 2);
    expect(extreme.curves.master[0][1]).toBe(1);
  });

  test("mixed curves keep their points far enough apart to drag", () => {
    const mine = edited({
      curves: {
        ...defaultAdjustments().curves,
        master: [
          [0, 0],
          [0.26, 0.3],
          [1, 1],
        ],
      },
    });
    const preset = { curves: { ...mine.curves, master: [[0, 0.1], [0.25, 0.2], [1, 0.9]] as [number, number][] } };
    for (const amount of [0.2, 0.5, 0.8, 1.5]) {
      const xs = withPreset(mine, preset, amount).curves.master.map((point) => point[0]);
      for (let i = 1; i < xs.length; i++) expect(xs[i] - xs[i - 1]).toBeGreaterThanOrEqual(POINT_GAP);
      // The point kept is the one from the curve the result is nearer to.
      expect(xs).toContain(amount < 0.5 ? 0.26 : 0.25);
    }
  });

  test("the colour mixer moves band by band", () => {
    const mixer = defaultAdjustments().mixer.map((band, i) => (i === 5 ? { hue: -30, saturation: 20, luminance: -70 } : band));
    const half = withPreset(defaultAdjustments(), { mixer }, 0.5).mixer;
    expect(half[5]).toEqual({ hue: -15, saturation: 10, luminance: -35 });
    expect(half[0]).toEqual({ hue: 0, saturation: 0, luminance: 0 });
    expect(withPreset(defaultAdjustments(), { mixer }, 2).mixer[5]).toEqual({ hue: -60, saturation: 40, luminance: -100 });
  });
});
