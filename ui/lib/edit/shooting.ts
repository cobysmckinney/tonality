import type { ImageMeta } from "$lib/bindings/ImageMeta";

/** Shutter speed as cameras show it: `1/250 s`, `1/4 s`, `0.6 s`, `2 s`, `30 s`. */
export function shutter(seconds: number): string {
  // Below about 0.4 s cameras name a fraction; above it, decimal seconds.
  if (seconds < 0.4) return `1/${Math.round(1 / seconds)} s`;
  const s = seconds < 10 ? Math.round(seconds * 10) / 10 : Math.round(seconds);
  return `${s} s`;
}

/** `f/2.8`, `f/8`, `f/11`. */
export function aperture(f: number): string {
  return `f/${f < 10 ? Math.round(f * 10) / 10 : Math.round(f)}`;
}

/**
 * One line of shooting settings: ISO, shutter speed, aperture and focal length, each left out
 * when the file didn't record it (or recorded nonsense).
 */
export function shootingInfo(
  meta: Pick<ImageMeta, "iso" | "exposureTime" | "fNumber" | "focalLength">,
): string[] {
  const ok = (v: number | null): v is number => v !== null && Number.isFinite(v) && v > 0;
  const out: string[] = [];
  if (ok(meta.iso)) out.push(`ISO ${Math.round(meta.iso)}`);
  if (ok(meta.exposureTime)) out.push(shutter(meta.exposureTime));
  if (ok(meta.fNumber)) out.push(aperture(meta.fNumber));
  if (ok(meta.focalLength)) out.push(`${Math.round(meta.focalLength)} mm`);
  return out;
}
