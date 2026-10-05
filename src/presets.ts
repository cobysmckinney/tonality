import {
  Adjustments,
  Band,
  CurveChannel,
  CurvePoints,
  curveSampler,
  DEFAULTS,
  GEOMETRY,
  POINT_GAP,
  rangeOf,
  same,
  SECTIONS,
  SliderKey,
} from "./adjustments";

/**
 * Part of a recipe: values for the settings a preset covers. Applying the
 * preset changes those and leaves the rest of the photo's edits alone. The
 * crop and the other framing settings are never part of one, and nor are masks.
 */
export type PresetSettings = Partial<Omit<Adjustments, (typeof GEOMETRY)[number] | "masks">>;

/** A named look. Mirrors `Preset` in the backend's presets.rs. */
export interface Preset {
  /** Negative for the built-in ones. */
  id: number;
  name: string;
  /** The heading a built-in preset is listed under; null for one of your own. */
  group: string | null;
  settings: PresetSettings;
}

/** The groups of settings a preset of your own can cover: the adjust panel's sections. */
export const PRESET_SECTIONS = SECTIONS.filter((section) => section.title !== "Crop");

const keysOf = (settings: PresetSettings) => Object.keys(settings) as (keyof PresetSettings)[];

/** The sections in which a recipe differs from as shot: what a new preset covers unless you say otherwise. */
export function changedSections(adjustments: Adjustments): string[] {
  return PRESET_SECTIONS.filter((section) => section.keys.some((key) => !same(adjustments[key], DEFAULTS[key]))).map(
    (section) => section.title,
  );
}

/** What a preset covering these sections would hold, taken from a recipe. */
export function settingsFrom(adjustments: Adjustments, sections: string[]): PresetSettings {
  const keys = PRESET_SECTIONS.filter((section) => sections.includes(section.title)).flatMap((section) => section.keys);
  return Object.fromEntries(keys.map((key) => [key, adjustments[key]]));
}

/** True if every one of these values is what the recipe has. */
export function holds(adjustments: Adjustments, values: Partial<Adjustments>): boolean {
  return (Object.keys(values) as (keyof Adjustments)[]).every((key) => same(adjustments[key], values[key]));
}

/** True if the recipe has the preset's values everywhere the preset covers, and the preset does something. */
export const isOn = (preset: Preset, adjustments: Adjustments) =>
  holds(adjustments, preset.settings) && !holds(DEFAULTS, preset.settings);

const mix = (from: number, to: number, amount: number) => from + (to - from) * amount;
const clamp = (value: number, low: number, high: number) => Math.min(high, Math.max(low, value));

function mixSlider(key: SliderKey, from: number, to: number, amount: number): number {
  const { min, max, step } = rangeOf(key);
  // Whole steps, without the stray digits that multiplying by 0.01 leaves behind.
  const decimals = Math.max(0, -Math.floor(Math.log10(step)));
  return Number(clamp(Math.round(mix(from, to, amount) / step) * step, min, max).toFixed(decimals));
}

/**
 * A curve part-way between two others. Curves are mixed as the lines they
 * draw, not point by point: the result has a point wherever either curve
 * has one, at the height between the two lines there. Where a point of one
 * is closer to a point of the other than the curve editor allows, only one
 * is kept, from the curve the result is nearer to.
 */
function mixCurve(from: CurvePoints, to: CurvePoints, amount: number): CurvePoints {
  const [a, b] = [curveSampler(from), curveSampler(to)];
  const nearer = amount >= 0.5 ? to : from;
  const positions = [...from, ...to]
    .map((point) => ({ x: point[0], preferred: nearer.includes(point) }))
    .sort((p, q) => p.x - q.x);
  const kept: typeof positions = [];
  for (const point of positions) {
    const last = kept.at(-1);
    if (!last || point.x - last.x >= POINT_GAP) kept.push(point);
    // Moving the last one along to here only widens its gap to the one before.
    else if (point.preferred && !last.preferred) kept[kept.length - 1] = point;
  }
  const round = (value: number) => Math.round(value * 1e4) / 1e4;
  return kept.map(({ x }) => [x, round(clamp(mix(a(x), b(x), amount), 0, 1))]);
}

/**
 * The values a preset gives the settings it covers, at some strength:
 * `amount` 0 leaves them as they are in `base`, 1 is the preset itself, and
 * past 1 carries on in the same direction.
 */
export function blend(base: Adjustments, settings: PresetSettings, amount = 1): Partial<Adjustments> {
  if (amount === 1) return { ...settings };
  if (amount === 0) return Object.fromEntries(keysOf(settings).map((key) => [key, base[key]]));
  const result: Record<string, unknown> = {};
  for (const key of keysOf(settings)) {
    if (key === "curves") {
      const channels = Object.keys(base.curves) as CurveChannel[];
      result.curves = Object.fromEntries(
        channels.map((channel) => [channel, mixCurve(base.curves[channel], settings.curves![channel], amount)]),
      );
    } else if (key === "mixer") {
      const part = (from: number, to: number) => clamp(Math.round(mix(from, to, amount)), -100, 100);
      result.mixer = base.mixer.map((band, i): Band => {
        const to = settings.mixer![i];
        return {
          hue: part(band.hue, to.hue),
          saturation: part(band.saturation, to.saturation),
          luminance: part(band.luminance, to.luminance),
        };
      });
    } else {
      result[key] = mixSlider(key, base[key], settings[key]!, amount);
    }
  }
  return result;
}

/** A recipe with a preset laid over it. */
export const withPreset = (base: Adjustments, settings: PresetSettings, amount = 1): Adjustments => ({
  ...base,
  ...blend(base, settings, amount),
});
