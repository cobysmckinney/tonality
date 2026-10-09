/**
 * White balance in Kelvin.
 *
 * Recipes keep Temperature and Tint relative to the camera's own white
 * balance: 0 is as shot, so a recipe, a preset or a paste means the same on
 * any photo. The develop shader turns each 100 of Temperature into red and
 * blue 1.2 stops apart, and each 100 of Tint into green 0.35 stops against
 * them (`white.rs`, `develop.wgsl`).
 *
 * A RAW's file says what light the camera balanced for, and how the camera
 * sees other light (`Camera`, from `white::camera`). From it the sliders
 * read as other RAW editors' do: the colour temperature of the light in
 * Kelvin, and its tint off the line of glowing-body colours (lamplight to
 * daylight to blue sky), positive for greener light. Telling the editor the
 * light was bluer (a higher Kelvin) warms the photo to make up for it.
 *
 * Moving Temperature walks along that line: each Kelvin sets the gains that
 * turn a white card lit that way grey, as the camera saw it. Tint moves off
 * the line. A scan of a negative, or a JPEG, has no light of its own to
 * name, so its sliders stay a plain warmer/cooler, greener/more magenta scale.
 */

export type Xy = [number, number];
type Vector = [number, number, number];
export type Matrix = [Vector, Vector, Vector];

/** How a RAW's camera white balanced it, from the backend (`white::Camera`). */
export interface Camera {
  /** The light the camera balanced for, as a CIE xy chromaticity. */
  asShot: Xy;
  /**
   * For each light the camera's colour was measured under, by colour
   * temperature: the matrix from a light's XYZ to the linear sRGB a white
   * card under it has in the photo as opened.
   */
  lights: { kelvin: number; matrix: Matrix }[];
}

/** What the sliders hold: relative to as shot, in slider units. */
export interface Relative {
  temperature: number;
  tint: number;
}

/** What a RAW's sliders show: the light's colour temperature and tint. */
export interface Light {
  kelvin: number;
  tint: number;
}

/** Where the Kelvin slider runs at most, as other editors have it. */
export const KELVIN_RANGE = { min: 2000, max: 50000 };
/** How far Tint goes either way on a RAW. */
export const TINT_RANGE = 150;
/** How far either slider goes either way where it isn't in Kelvin. */
export const RELATIVE_RANGE = 100;

/** Stops red and blue move apart, and green against them, per 100 of Temperature and Tint. White.rs. */
const TEMPERATURE_STOPS = 1.2;
const TINT_STOPS = 0.35;
/** A light whose card would need one channel this far below the others can't be made up for. */
const DARKEST_SHARE = 0.05;
/** The furthest the shader takes the photo's own Temperature and Tint either way: `MAX_WHITE_BALANCE` in edit.rs. */
const MOST = 2000;

const multiply = (m: Matrix, v: Vector): Vector =>
  [0, 1, 2].map((r) => m[r][0] * v[0] + m[r][1] * v[1] + m[r][2] * v[2]) as Vector;

const toXyz = ([x, y]: Xy): Vector => [x / y, 1, (1 - x - y) / y];

/** The colour of a body glowing at `kelvin`, in CIE 1960 uv (Krystek's fit). */
function glowing(kelvin: number): [number, number] {
  const t = kelvin;
  return [
    (0.860117757 + 1.54118254e-4 * t + 1.28641212e-7 * t * t) / (1 + 8.42420235e-4 * t + 7.08145163e-7 * t * t),
    (0.317398726 + 4.22806245e-5 * t + 4.20481691e-8 * t * t) / (1 - 2.89741816e-5 * t + 1.61456053e-7 * t * t),
  ];
}

/** Along the glowing-body line towards higher Kelvin, and off it towards green, both of unit length. */
function directions(kelvin: number): { along: [number, number]; green: [number, number] } {
  const [a, b] = [glowing(kelvin * 0.999), glowing(kelvin * 1.001)];
  const [du, dv] = [b[0] - a[0], b[1] - a[1]];
  const length = Math.hypot(du, dv);
  const along: [number, number] = [du / length, dv / length];
  // Green lies on the side of higher v.
  const green: [number, number] = along[0] > 0 ? [-along[1], along[0]] : [along[1], -along[0]];
  return { along, green };
}

/** A light's chromaticity (xy) from its colour temperature and tint (`lightOf`). */
export function chromaticity({ kelvin, tint }: Light): Xy {
  const [u, v] = glowing(kelvin);
  const { green } = directions(kelvin);
  const off = tint / 3000;
  const [uu, vv] = [u + green[0] * off, v + green[1] * off];
  const d = 2 * uu - 8 * vv + 4;
  return [(3 * uu) / d, (2 * vv) / d];
}

/**
 * A light's colour temperature and tint from its chromaticity (xy). Tint
 * counts 3000ths of the distance off the line of glowing bodies (in CIE 1960
 * uv), positive for greener light, as other editors count it: making up for
 * a green light takes magenta.
 */
export function lightOf([x, y]: Xy): Light {
  const d = -2 * x + 12 * y + 3;
  const [u, v] = [(4 * x) / d, (6 * y) / d];
  // The nearest point on the line: where the way to it is square to the line.
  const along = (mired: number) => {
    const kelvin = 1e6 / mired;
    const [lu, lv] = glowing(kelvin);
    const { along } = directions(kelvin);
    return (u - lu) * along[0] + (v - lv) * along[1];
  };
  let [low, high] = [10, 1000];
  const lowSign = Math.sign(along(low));
  for (let i = 0; i < 60; i++) {
    const middle = (low + high) / 2;
    if (Math.sign(along(middle)) === lowSign) low = middle;
    else high = middle;
  }
  const kelvin = 1e6 / ((low + high) / 2);
  const [lu, lv] = glowing(kelvin);
  const { green } = directions(kelvin);
  return { kelvin, tint: ((u - lu) * green[0] + (v - lv) * green[1]) * 3000 };
}

/**
 * The colour a white card lit by `light` has in the photo as opened, in
 * linear sRGB: through the camera's matrix for that light, between the two
 * measured either side of it by inverse temperature, as the DNG
 * specification has it.
 */
export function cardColour(light: Light, camera: Camera): Vector {
  const lights = [...camera.lights].sort((a, b) => a.kelvin - b.kelvin);
  const above = lights.findIndex((l) => l.kelvin >= light.kelvin);
  let matrix: Matrix;
  if (above === 0 || lights.length === 1) matrix = lights[0].matrix;
  else if (above < 0) matrix = lights[lights.length - 1].matrix;
  else {
    const [low, high] = [lights[above - 1], lights[above]];
    const weight = (1 / light.kelvin - 1 / high.kelvin) / (1 / low.kelvin - 1 / high.kelvin);
    matrix = low.matrix.map((row, r) => row.map((v, c) => weight * v + (1 - weight) * high.matrix[r][c])) as Matrix;
  }
  return multiply(matrix, toXyz(chromaticity(light)));
}

/** The slider values whose gains turn `colour` (linear sRGB) grey: what the eyedropper sets (`white::neutralising`). */
export function neutralising([r, g, b]: Vector): Relative {
  return {
    temperature: (Math.log2(b / r) / TEMPERATURE_STOPS) * 100,
    tint: (Math.log2(g / Math.sqrt(r * b)) / TINT_STOPS) * 100,
  };
}

/** Converts between a RAW's sliders in Kelvin and the relative values a recipe keeps. */
export interface KelvinScale {
  /** What the camera balanced for. */
  asShot: Light;
  /** How far the Kelvin slider runs for this photo. Lights far from as shot can need more than sRGB holds. */
  lowest: number;
  highest: number;
  toRelative: (light: Light) => Relative;
  toLight: (relative: Relative) => Light;
}

/** The Kelvin sliders of a RAW whose camera balanced it as `camera` says. */
export function kelvinScale(camera: Camera): KelvinScale {
  const shot = lightOf(camera.asShot);
  // Temperature is set to what picking a white card would set, lit by a
  // light of that Kelvin and the as-shot tint. Tint then adds what that
  // much more tint adds to the card's green. Kept apart like this, each
  // slider moves the photo the same way all along (near the warmest light
  // a little tint shifts a card's red and blue too, which would turn
  // Temperature back on itself), and the eyedropper names a light of the
  // as-shot tint exactly, others closely.
  const card = (kelvin: number, tint = shot.tint) =>
    cardColour({ kelvin, tint }, camera).map((c) => Math.max(c, 1e-6)) as Vector;
  const along = (kelvin: number) => neutralising(card(kelvin));
  const perTint = (kelvin: number) =>
    (neutralising(card(kelvin, shot.tint + 5)).tint - neutralising(card(kelvin, shot.tint - 5)).tint) / 10;
  const origin = along(shot.kelvin);

  // A card far enough from as shot needs more than sRGB holds to make up for.
  const usable = (kelvin: number) => {
    const colour = card(kelvin);
    return Math.min(...colour) / Math.max(...colour) >= DARKEST_SHARE;
  };
  let lowest = KELVIN_RANGE.min;
  while (lowest < shot.kelvin && !usable(lowest)) lowest *= 1.02;
  let highest = KELVIN_RANGE.max;
  while (highest > shot.kelvin && !usable(highest)) highest /= 1.02;
  lowest = Math.round(Math.min(lowest, shot.kelvin));
  highest = Math.round(Math.max(highest, shot.kelvin));

  // Only the far corners (the warmest light, with the most tint) go past what the shader takes.
  // Rounded, so going back to as shot lands on exactly 0.
  const limit = (v: number) => Math.round(Math.min(MOST, Math.max(-MOST, v)) * 1e4) / 1e4 || 0;
  const toRelative = ({ kelvin, tint }: Light): Relative => {
    const at = Math.min(highest, Math.max(lowest, kelvin));
    const [here, per] = [along(at), perTint(at)];
    return {
      temperature: limit(here.temperature - origin.temperature),
      tint: limit(here.tint + (tint - shot.tint) * per - origin.tint),
    };
  };

  const toLight = ({ temperature, tint }: Relative): Light => {
    if (temperature === 0 && tint === 0) return shot;
    let kelvin = shot.kelvin;
    if (temperature !== 0) {
      // Temperature rises with Kelvin.
      let [low, high] = [1e6 / highest, 1e6 / lowest];
      for (let i = 0; i < 50; i++) {
        const middle = (low + high) / 2;
        if (along(1e6 / middle).temperature > temperature + origin.temperature) low = middle;
        else high = middle;
      }
      kelvin = 1e6 / ((low + high) / 2);
    }
    return { kelvin, tint: shot.tint + (tint + origin.tint - along(kelvin).tint) / perTint(kelvin) };
  };

  return { asShot: shot, lowest, highest, toRelative, toLight };
}

/** Brings a picked white balance inside what the sliders can show. */
export function withinReach(picked: Relative, scale: KelvinScale | null): Relative {
  if (!scale) {
    const clamp = (v: number) => Math.min(RELATIVE_RANGE, Math.max(-RELATIVE_RANGE, v));
    return { temperature: clamp(picked.temperature), tint: clamp(picked.tint) };
  }
  const { kelvin, tint } = scale.toLight(picked);
  return scale.toRelative({
    kelvin: Math.min(scale.highest, Math.max(scale.lowest, kelvin)),
    tint: Math.min(TINT_RANGE, Math.max(-TINT_RANGE, tint)),
  });
}

/** A colour temperature as the slider shows it: "5200 K". */
export function formatKelvin(kelvin: number): string {
  const step = kelvin < 10000 ? 10 : 100;
  return `${Math.round(kelvin / step) * step} K`;
}

/** The Kelvin slider runs evenly in mireds (a million over Kelvin), as the eye sees steps of warmth; right is higher Kelvin. */
export const kelvinToPosition = (kelvin: number) => -1e6 / kelvin;
export const positionToKelvin = (position: number) => -1e6 / position;

const scales = new WeakMap<Camera, KelvinScale>();

/**
 * The Kelvin sliders for a photo, worked out once per photo: null for a
 * JPEG, or a scan of a negative, whose sliders aren't in Kelvin.
 */
export function sliderScale(camera: Camera | null, negative: boolean): KelvinScale | null {
  if (!camera || negative || camera.lights.length === 0) return null;
  let scale = scales.get(camera);
  if (!scale) scales.set(camera, (scale = kelvinScale(camera)));
  return scale;
}
