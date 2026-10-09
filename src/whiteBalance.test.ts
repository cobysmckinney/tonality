import { describe, expect, test } from "bun:test";
import {
  Camera,
  cardColour,
  chromaticity,
  formatKelvin,
  kelvinScale,
  lightOf,
  Matrix,
  neutralising,
  RELATIVE_RANGE,
  withinReach,
  Xy,
} from "./whiteBalance";

const D65: Xy = [0.31271, 0.32902];
const TUNGSTEN: Xy = [0.44757, 0.40745];

const SRGB_TO_XYZ: Matrix = [
  [0.4124564, 0.3575761, 0.1804375],
  [0.2126729, 0.7151522, 0.072175],
  [0.0193339, 0.119192, 0.9503041],
];
/** A Canon EOS 5D Mark III's colour matrices (XYZ to camera), under tungsten and daylight. */
const CANON_A: Matrix = [
  [0.7234, -0.1413, -0.06],
  [-0.3631, 1.115, 0.285],
  [-0.0382, 0.1335, 0.6437],
];
const CANON_D65: Matrix = [
  [0.6722, -0.0635, -0.0963],
  [-0.4287, 1.246, 0.2028],
  [-0.0908, 0.2162, 0.5668],
];

const times = (a: Matrix, b: Matrix) => a.map((row) => [0, 1, 2].map((c) => row[0] * b[0][c] + row[1] * b[1][c] + row[2] * b[2][c])) as Matrix;
const apply = (m: Matrix, v: number[]) => m.map((row) => row[0] * v[0] + row[1] * v[1] + row[2] * v[2]);
function inverse(m: Matrix): Matrix {
  const [[a, b, c], [d, e, f], [g, h, i]] = m;
  const det = a * (e * i - f * h) - b * (d * i - f * g) + c * (d * h - e * g);
  return [
    [(e * i - f * h) / det, (c * h - b * i) / det, (b * f - c * e) / det],
    [(f * g - d * i) / det, (a * i - c * g) / det, (c * d - a * f) / det],
    [(d * h - e * g) / det, (b * g - a * h) / det, (a * e - b * d) / det],
  ];
}

/** What `white::camera` says of a 5D Mark III balanced for `asShot`. */
function canon(asShot: Xy): Camera {
  const kelvin = lightOf(asShot).kelvin;
  const weight = Math.min(1, Math.max(0, (1 / kelvin - 1 / 6504) / (1 / 2856 - 1 / 6504)));
  const there = CANON_A.map((row, r) => row.map((v, c) => weight * v + (1 - weight) * CANON_D65[r][c])) as Matrix;
  const [x, y] = asShot;
  const neutral = apply(there, [x / y, 1, (1 - x - y) / y]);
  const balance = neutral.map((n) => neutral[1] / n);
  const toCamera = times(CANON_D65, SRGB_TO_XYZ).map((row) => row.map((v) => v / (row[0] + row[1] + row[2]))) as Matrix;
  const toSrgb = inverse(toCamera);
  const balanced = (m: Matrix) => m.map((row, r) => row.map((v) => v * balance[r])) as Matrix;
  return {
    asShot,
    lights: [
      { kelvin: 2856, matrix: times(toSrgb, balanced(CANON_A)) },
      { kelvin: 6504, matrix: times(toSrgb, balanced(CANON_D65)) },
    ],
  };
}

describe("colour temperature", () => {
  test("names the standard lights", () => {
    const daylight = lightOf(D65);
    expect(Math.abs(daylight.kelvin - 6504)).toBeLessThan(30);
    // Daylight sits a little on the green side of a glowing body.
    expect(daylight.tint).toBeGreaterThan(5);
    expect(daylight.tint).toBeLessThan(15);
    const tungsten = lightOf(TUNGSTEN);
    expect(Math.abs(tungsten.kelvin - 2856)).toBeLessThan(20);
    expect(Math.abs(tungsten.tint)).toBeLessThan(2);
  });

  test("goes back and forth between Kelvin and chromaticity", () => {
    for (const kelvin of [2000, 3200, 5200, 9000, 25000]) {
      for (const tint of [-100, 0, 40]) {
        const back = lightOf(chromaticity({ kelvin, tint }));
        expect(Math.abs(back.kelvin - kelvin) / kelvin).toBeLessThan(1e-3);
        expect(Math.abs(back.tint - tint)).toBeLessThan(0.1);
      }
    }
  });

  test("greener light has a higher tint", () => {
    const green = chromaticity({ kelvin: 5000, tint: 30 });
    const plain = chromaticity({ kelvin: 5000, tint: 0 });
    expect(green[1]).toBeGreaterThan(plain[1]);
  });
});

describe("a RAW's Kelvin sliders", () => {
  const camera = canon(chromaticity({ kelvin: 5200, tint: 4 }));
  const scale = kelvinScale(camera);

  test("show what the camera balanced for until moved", () => {
    expect(Math.abs(scale.asShot.kelvin - 5200)).toBeLessThan(1);
    expect(Math.abs(scale.asShot.tint - 4)).toBeLessThan(0.01);
    expect(scale.toLight({ temperature: 0, tint: 0 })).toEqual(scale.asShot);
    const shot = scale.toRelative(scale.asShot);
    expect(Math.abs(shot.temperature)).toBeLessThan(1e-6);
    expect(Math.abs(shot.tint)).toBeLessThan(1e-6);
  });

  test("keep what was set", () => {
    for (const light of [
      { kelvin: 2400, tint: 0 },
      { kelvin: 3000, tint: -20 },
      { kelvin: 7500, tint: 60 },
      { kelvin: 40000, tint: -150 },
    ]) {
      const back = scale.toLight(scale.toRelative(light));
      expect(Math.abs(back.kelvin - light.kelvin) / light.kelvin).toBeLessThan(1e-3);
      expect(Math.abs(back.tint - light.tint)).toBeLessThan(0.05);
    }
  });

  test("warm the photo for a higher Kelvin, and turn it magenta for a higher tint", () => {
    expect(scale.toRelative({ kelvin: 7000, tint: 4 }).temperature).toBeGreaterThan(0);
    expect(scale.toRelative({ kelvin: 3500, tint: 4 }).temperature).toBeLessThan(0);
    expect(scale.toRelative({ kelvin: 5200, tint: 30 }).tint).toBeGreaterThan(0);
  });

  test("name the light a white card was lit by when it is picked", () => {
    // The eyedropper makes the card grey; the sliders then say what light
    // that was: exactly for light of the as-shot tint, and close for other
    // light (a few mireds; more towards the ends of the scale, where a
    // little tint also shifts red against blue).
    const mireds = (kelvin: number) => 1e6 / kelvin;
    for (const shot of [3000, 5200, 8000]) {
      const each = canon(chromaticity({ kelvin: shot, tint: 4 }));
      const sliders = kelvinScale(each);
      for (const kelvin of [2800, 4000, 6500, 12000]) {
        for (const tint of [4, -6, 14]) {
          const picked = sliders.toLight(neutralising(cardColour({ kelvin, tint }, each)));
          const close = kelvin < 3500 || kelvin > 10000 ? 25 : 8;
          expect(Math.abs(mireds(picked.kelvin) - mireds(kelvin))).toBeLessThan(tint === 4 ? 0.01 : close);
          expect(Math.abs(picked.tint - tint)).toBeLessThan(tint === 4 ? 0.01 : close / 4);
        }
      }
    }
  });

  test("reach the lights a photo is shot under, from any white balance", () => {
    for (const shot of [2800, 3200, 5200, 6500, 10000]) {
      const each = kelvinScale(canon(chromaticity({ kelvin: shot, tint: 0 })));
      expect(each.lowest).toBeLessThanOrEqual(2800);
      expect(each.highest).toBeGreaterThanOrEqual(12000);
    }
  });

  test("stay inside what the shader takes, without cutting off the lights photos are taken under", () => {
    for (const shot of [2500, 3200, 6500, 20000]) {
      for (const shotTint of [-40, 0, 40]) {
        const each = kelvinScale(canon(chromaticity({ kelvin: shot, tint: shotTint })));
        for (const kelvin of [each.lowest, 2700, 6500, 25000, each.highest]) {
          for (const tint of [-150, -60, 60, 150]) {
            const { temperature, tint: t } = each.toRelative({ kelvin, tint });
            expect(Math.abs(temperature)).toBeLessThanOrEqual(2000);
            expect(Math.abs(t)).toBeLessThanOrEqual(2000);
            if (Math.abs(tint) <= 60 && kelvin > each.lowest && kelvin < each.highest) {
              expect(Math.abs(temperature)).toBeLessThan(1999);
              expect(Math.abs(t)).toBeLessThan(1999);
            }
          }
        }
      }
    }
  });
});

describe("a picked white balance", () => {
  test("stays inside the plain sliders where there is no Kelvin", () => {
    expect(withinReach({ temperature: -260, tint: 40 }, null)).toEqual({ temperature: -RELATIVE_RANGE, tint: 40 });
  });

  test("stays inside the Kelvin sliders on a RAW", () => {
    const scale = kelvinScale(canon(chromaticity({ kelvin: 5200, tint: 0 })));
    const reached = scale.toLight(withinReach({ temperature: 1900, tint: -1900 }, scale));
    expect(Math.abs(reached.kelvin - scale.highest)).toBeLessThan(scale.highest * 0.002);
    expect(Math.abs(reached.tint + 150)).toBeLessThan(0.1);
    const fine = { temperature: -40, tint: 12 };
    const kept = withinReach(fine, scale);
    expect(Math.abs(kept.temperature - fine.temperature)).toBeLessThan(0.01);
    expect(Math.abs(kept.tint - fine.tint)).toBeLessThan(0.01);
  });
});

test("Kelvin reads in round numbers", () => {
  expect(formatKelvin(5203.4)).toBe("5200 K");
  expect(formatKelvin(2856)).toBe("2860 K");
  expect(formatKelvin(23456)).toBe("23500 K");
});
