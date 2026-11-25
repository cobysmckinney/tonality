// Geometry for the navigator: a small overview of the whole frame with a rectangle for the part
// the canvas shows while zoomed in. Pure functions, in whatever pixel units the caller uses.

/** A rectangle: in pixels, or in uv (0..1) of the frame, depending on use. */
export interface Box {
  x: number;
  y: number;
  w: number;
  h: number;
}

/** The overview's size: the frame's aspect ratio, `edge` on the long side. */
export function overviewSize(frameW: number, frameH: number, edge: number) {
  const s = edge / Math.max(frameW, frameH, 1e-9);
  return { w: Math.max(1, Math.round(frameW * s)), h: Math.max(1, Math.round(frameH * s)) };
}

/**
 * The part of the frame a pane shows (uv, clipped to the frame), given where the whole frame sits
 * on that pane (`frame`, in the pane's pixels) and the pane's size.
 */
export function visibleRect(frame: Box, paneW: number, paneH: number): Box {
  if (frame.w <= 0 || frame.h <= 0) return { x: 0, y: 0, w: 1, h: 1 };
  const x0 = clamp01(-frame.x / frame.w);
  const y0 = clamp01(-frame.y / frame.h);
  const x1 = clamp01((paneW - frame.x) / frame.w);
  const y1 = clamp01((paneH - frame.y) / frame.h);
  return { x: x0, y: y0, w: Math.max(0, x1 - x0), h: Math.max(0, y1 - y0) };
}

/** Does the overview point (px, py) fall inside the visible rectangle? */
export function inVisible(px: number, py: number, size: { w: number; h: number }, vis: Box) {
  const u = px / size.w;
  const v = py / size.h;
  return u >= vis.x && u <= vis.x + vis.w && v >= vis.y && v <= vis.y + vis.h;
}

/**
 * Where a press at overview point (px, py) holds the visible rectangle: the offset (uv) from the
 * rectangle's centre when the press lands on it (so dragging doesn't jump), zero otherwise (a
 * press elsewhere brings the rectangle's centre there).
 */
export function grabOffset(px: number, py: number, size: { w: number; h: number }, vis: Box) {
  if (!inVisible(px, py, size, vis)) return { x: 0, y: 0 };
  return { x: px / size.w - (vis.x + vis.w / 2), y: py / size.h - (vis.y + vis.h / 2) };
}

/**
 * The view centre (uv of the frame) that puts the visible rectangle under overview point
 * (px, py), less the grab offset, kept so the rectangle stays inside the frame.
 */
export function centerAt(
  px: number,
  py: number,
  size: { w: number; h: number },
  vis: Box,
  offset = { x: 0, y: 0 },
) {
  const keep = (c: number, extent: number) =>
    extent >= 1 ? 0.5 : Math.min(1 - extent / 2, Math.max(extent / 2, c));
  return {
    x: keep(px / size.w - offset.x, vis.w),
    y: keep(py / size.h - offset.y, vis.h),
  };
}

function clamp01(v: number) {
  return Math.min(1, Math.max(0, v));
}
