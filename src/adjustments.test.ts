import { describe, expect, test } from "bun:test";
import { Adjustments, CurvePoints, curveSampler, defaultAdjustments, describeChange, noLocalAdjustments } from "./adjustments";
import samples from "./curve-samples.json";

const edited = (change: Partial<Adjustments>): Adjustments => ({ ...defaultAdjustments(), ...change });
const bent: CurvePoints = [
  [0, 0],
  [0.25, 0.18],
  [0.75, 0.85],
  [1, 1],
];

describe("history step names", () => {
  test("nothing changed, or everything put back", () => {
    expect(describeChange(edited({ exposure: 1 }), edited({ exposure: 1 }))).toBe("No change");
    expect(describeChange(edited({ exposure: 1, rotation: 1, contrast: 20 }), defaultAdjustments())).toBe("Reset all");
  });

  test("one slider says what it is now", () => {
    const start = defaultAdjustments();
    expect(describeChange(start, edited({ exposure: 0.5 }))).toBe("Exposure +0.50");
    expect(describeChange(start, edited({ exposure: -1.25 }))).toBe("Exposure −1.25");
    expect(describeChange(start, edited({ contrast: -20 }))).toBe("Contrast −20");
    expect(describeChange(start, edited({ noiseReduction: 35 }))).toBe("Noise reduction +35");
    expect(describeChange(edited({ shadows: 10, vibrance: 5 }), edited({ shadows: 0, vibrance: 5 }))).toBe("Shadows 0");
  });

  test("the order fields are written in doesn't count as a change", () => {
    const before = edited({ crop: { x: 0.4, y: 0.5, width: 0.6, height: 0.8 } });
    const after = edited({ crop: { height: 0.8, width: 0.6, y: 0.5, x: 0.4 } });
    expect(describeChange(before, after)).toBe("No change");
  });

  test("framing changes are named by what was done", () => {
    const start = edited({ exposure: 1 });
    expect(describeChange(start, { ...start, rotation: 1 })).toBe("Rotate");
    expect(describeChange(start, { ...start, flipHorizontal: true })).toBe("Flip");
    expect(describeChange(start, { ...start, flipVertical: true })).toBe("Flip");
    expect(describeChange(start, { ...start, straighten: 2.5 })).toBe("Straighten +2.5°");
    expect(describeChange(start, { ...start, crop: { x: 0.5, y: 0.5, width: 0.8, height: 0.8 } })).toBe("Crop");
    // A crop dragged while straightening is still named for the straightening.
    expect(describeChange(start, { ...start, straighten: -4, crop: { x: 0.5, y: 0.5, width: 0.9, height: 0.9 } })).toBe(
      "Straighten −4.0°",
    );
    const framed = { ...start, rotation: 3, straighten: 4, crop: { x: 0.4, y: 0.5, width: 0.5, height: 0.5 } };
    expect(describeChange(framed, start)).toBe("Reset crop");
  });

  test("curves name the channel when only one colour changed", () => {
    const start = defaultAdjustments();
    const curves = (change: Partial<Adjustments["curves"]>) => edited({ curves: { ...start.curves, ...change } });
    expect(describeChange(start, curves({ master: bent }))).toBe("Tone curve");
    expect(describeChange(start, curves({ red: bent }))).toBe("Red curve");
    expect(describeChange(start, curves({ blue: bent }))).toBe("Blue curve");
    expect(describeChange(start, curves({ red: bent, green: bent }))).toBe("Tone curve");
    expect(describeChange(curves({ red: bent, master: bent }), start)).toBe("Reset all");
    expect(describeChange(edited({ exposure: 1, curves: { ...start.curves, red: bent } }), edited({ exposure: 1 }))).toBe(
      "Reset curve",
    );
  });

  test("the color mixer names the band when only one changed", () => {
    const start = edited({ exposure: 1 });
    const mixer = (bands: number[]) => ({
      ...start,
      mixer: start.mixer.map((band, i) => (bands.includes(i) ? { ...band, hue: 10 } : band)),
    });
    expect(describeChange(start, mixer([1]))).toBe("Color mixer: oranges");
    expect(describeChange(start, mixer([7]))).toBe("Color mixer: magentas");
    expect(describeChange(start, mixer([0, 5]))).toBe("Color mixer");
    expect(describeChange(mixer([0, 5]), start)).toBe("Reset color mixer");
  });

  test("masks name their own steps; this is the fallback", () => {
    const start = defaultAdjustments();
    const mask = {
      id: 1,
      name: "Mask 1",
      visible: true,
      invert: false,
      parts: [],
      adjustments: noLocalAdjustments(),
    };
    expect(describeChange(edited({ exposure: 1 }), edited({ exposure: 1, masks: [mask] }))).toBe("Masks");
    expect(describeChange(start, edited({ masks: [mask] }))).toBe("Masks");
  });

  test("several at once is a section reset, or a count", () => {
    const kept = { saturation: 10 };
    expect(describeChange(edited({ ...kept, exposure: 1, contrast: 20, blacks: -5 }), edited(kept))).toBe("Reset light");
    expect(describeChange(edited({ exposure: 1, temperature: 20, tint: 5 }), edited({ exposure: 1 }))).toBe("Reset color");
    expect(describeChange(edited({ ...kept, vignette: -30, grain: 20 }), edited(kept))).toBe("Reset effects");
    // Changed, but not back to the defaults: a paste or the like.
    expect(describeChange(edited(kept), edited({ ...kept, exposure: 1, contrast: 20 }))).toBe("2 adjustments");
    expect(describeChange(edited({ ...kept, exposure: 1 }), edited({ ...kept, temperature: 10 }))).toBe("2 adjustments");
    expect(describeChange(edited(kept), edited({ ...kept, exposure: 1, vignette: -20, grain: 10 }))).toBe("3 adjustments");
  });
  test("film steps say what happened in plain words", () => {
    const none = defaultAdjustments();
    const colour = edited({ film: { kind: "colour", base: null, range: null } });
    expect(describeChange(none, colour)).toBe("Film: colour negative");
    expect(describeChange(colour, edited({ film: { kind: "blackAndWhite", base: null, range: null } }))).toBe(
      "Film: black and white negative",
    );
    expect(describeChange({ ...colour, exposure: 1 }, { ...none, exposure: 1 })).toBe("Film: none");
    const picked = edited({ film: { kind: "colour", base: [0.7, 0.4, 0.2], range: null } });
    expect(describeChange(colour, picked)).toBe("Film base picked");
    expect(describeChange(picked, colour)).toBe("Film base guessed");
    const roll = edited({ film: { kind: "colour", base: [0.7, 0.4, 0.2], range: { low: [0, 0, 0], high: [1, 1.1, 1.2] } } });
    expect(describeChange(colour, roll)).toBe("Balanced with the roll");
    expect(describeChange(roll, picked)).toBe("Balanced on its own");
    expect(describeChange(roll, defaultAdjustments())).toBe("Reset all");
  });
});

describe("curves", () => {
  test("a straight curve changes nothing", () => {
    const at = curveSampler([
      [0, 0],
      [1, 1],
    ]);
    for (let x = 0; x <= 1; x += 0.05) expect(at(x)).toBeCloseTo(x, 9);
  });

  test("a curve passes through its points without overshooting them", () => {
    const at = curveSampler(bent);
    for (const [x, y] of bent) expect(at(x)).toBeCloseTo(y, 9);
    let last = -1;
    for (let i = 0; i <= 1000; i++) {
      const y = at(i / 1000);
      expect(y).toBeGreaterThanOrEqual(last);
      last = y;
    }
  });

  test("a peak stays a peak, and flat stays flat", () => {
    const peak = curveSampler([
      [0, 0],
      [0.5, 0.9],
      [1, 0.2],
    ]);
    for (let i = 0; i <= 100; i++) expect(peak(i / 100)).toBeLessThanOrEqual(0.9);
    const flat = curveSampler([
      [0, 0],
      [0.3, 0.5],
      [0.7, 0.5],
      [1, 1],
    ]);
    for (let x = 0.3; x <= 0.7; x += 0.01) expect(flat(x)).toBeCloseTo(0.5, 9);
  });

  test("stays within black and white, and holds level past its end points", () => {
    const steep = curveSampler([
      [0, 0],
      [0.1, 1],
      [0.2, 0],
      [1, 1],
    ]);
    for (let i = 0; i <= 1000; i++) {
      expect(steep(i / 1000)).toBeGreaterThanOrEqual(0);
      expect(steep(i / 1000)).toBeLessThanOrEqual(1);
    }
    const short = curveSampler([
      [0.2, 0.1],
      [0.8, 0.95],
    ]);
    expect(short(0)).toBe(0.1);
    expect(short(1)).toBe(0.95);
  });

  // The same curves and values are checked against the backend's curve_table
  // in src-tauri/tests/curves.rs, so the line drawn is the curve applied.
  test("matches the curve the backend draws with", () => {
    for (const { name, points, samples: expected } of samples) {
      const at = curveSampler(points as CurvePoints);
      const got = expected.map((_, i) => at(i / (expected.length - 1)));
      got.forEach((y, i) => {
        if (Math.abs(y - expected[i]) > 1e-5) throw new Error(`${name} at ${i}/${expected.length - 1}: ${y}, not ${expected[i]}`);
      });
    }
  });
});
