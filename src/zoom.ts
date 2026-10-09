/**
 * Zoom and pan arithmetic for the editor's stage. The stage asks the backend
 * for exactly the pixels on screen, so a view is a zoom (screen pixels per
 * photo pixel) and the point of the picture at the middle of the stage.
 * Positions on the picture run 0..1 across and down the cropped frame.
 */
import type { Region } from "./api";
import type { Size } from "./crop";

/** How far in the picture is zoomed (screen pixels per photo pixel; null fits it to the stage) and where it's centred. */
export interface View {
  zoom: number | null;
  x: number;
  y: number;
}

export const FIT: View = { zoom: null, x: 0.5, y: 0.5 };
export const MAX_ZOOM = 4;

/** A box on screen, in CSS pixels. */
export interface Box {
  left: number;
  top: number;
  width: number;
  height: number;
}

/** Where the picture sits: the zoom that fits it, the zoom used, the size drawn in device pixels, and which part of it shows. */
export interface Placement {
  fit: number;
  zoom: number;
  width: number;
  height: number;
  region: Region;
}

const clamp = (v: number, low: number, high: number) => Math.min(high, Math.max(low, v));

/**
 * Places a picture `shown` photo pixels big on a stage `available` device
 * pixels big, at `zoom` (null fits it), centred on (x, y) as far as the
 * picture's edges allow. It is never drawn bigger than it is, or past the
 * stage.
 */
export function place(available: Size, shown: Size, zoom: number | null, x: number, y: number): Placement {
  const fit = Math.min(available.width / shown.width, available.height / shown.height, 1);
  const used = zoom === null ? fit : clamp(zoom, fit, MAX_ZOOM);
  const width = Math.max(1, Math.min(available.width, Math.round(shown.width * used)));
  const height = Math.max(1, Math.min(available.height, Math.round(shown.height * used)));
  const span = { width: width / (shown.width * used), height: height / (shown.height * used) };
  const region: Region = {
    x: clamp(x - span.width / 2, 0, 1 - span.width),
    y: clamp(y - span.height / 2, 0, 1 - span.height),
    ...span,
  };
  return { fit, zoom: used, width, height, region };
}

/**
 * The view after zooming to `zoom`, keeping the point of the picture under
 * the pointer where it is on the stage. `canvas` is where the picture is
 * drawn and `stage` the box around it, both in CSS pixels; `ratio` is device
 * pixels per CSS pixel and `frame` the picture's size in photo pixels. Zooming
 * out to the fit, or past it, fits the picture again.
 */
export function zoomedView(
  placed: Placement,
  frame: Size,
  ratio: number,
  canvas: Box,
  stage: Box,
  zoom: number | null,
  clientX: number,
  clientY: number,
): View {
  if (zoom === null || zoom <= placed.fit * 1.001) return FIT;
  // The point of the picture under the pointer…
  const { region } = placed;
  const u = region.x + clamp((clientX - canvas.left) / canvas.width, 0, 1) * region.width;
  const v = region.y + clamp((clientY - canvas.top) / canvas.height, 0, 1) * region.height;
  // …should sit at the same place on the stage after zooming.
  const next = clamp(zoom, placed.fit, MAX_ZOOM);
  const span = {
    width: Math.min(1, (stage.width * ratio) / (frame.width * next)),
    height: Math.min(1, (stage.height * ratio) / (frame.height * next)),
  };
  const fx = clamp((clientX - stage.left) / stage.width, 0, 1);
  const fy = clamp((clientY - stage.top) / stage.height, 0, 1);
  return { zoom: next, x: u + (0.5 - fx) * span.width, y: v + (0.5 - fy) * span.height };
}

/**
 * The view after dragging the picture by (dx, dy) CSS pixels: it follows the
 * pointer, and stops where its edges reach the stage's.
 */
export function pannedView(placed: Placement, frame: Size, ratio: number, dx: number, dy: number): View {
  const across = (dx * ratio) / (frame.width * placed.zoom);
  const down = (dy * ratio) / (frame.height * placed.zoom);
  const { region } = placed;
  return {
    zoom: placed.zoom,
    x: clamp(region.x + region.width / 2 - across, region.width / 2, 1 - region.width / 2),
    y: clamp(region.y + region.height / 2 - down, region.height / 2, 1 - region.height / 2),
  };
}

/**
 * Where the last frame drawn, of part `painted` of the picture, belongs on a
 * canvas now showing part `region` at `shown` CSS pixels: so while a zoom or
 * pan waits for its frame, the last one is scaled and moved rather than
 * stretched to fit.
 */
export const heldFrame = (painted: Region, region: Region, shown: Size): Box => ({
  left: ((painted.x - region.x) / region.width) * shown.width,
  top: ((painted.y - region.y) / region.height) * shown.height,
  width: (painted.width / region.width) * shown.width,
  height: (painted.height / region.height) * shown.height,
});
