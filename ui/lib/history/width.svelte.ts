// The Versions & History panel's width: dragged from its right edge, remembered per viewer.
const KEY = "tonality.edit.historyWidth";
export const MIN_WIDTH = 200;
export const MAX_WIDTH = 400;
export const DEFAULT_WIDTH = 240;

export const clampWidth = (px: number) =>
  Math.round(Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, Number.isFinite(px) ? px : DEFAULT_WIDTH)));

function stored(): number {
  try {
    const v = localStorage.getItem(KEY);
    return v === null ? DEFAULT_WIDTH : clampWidth(Number(v));
  } catch {
    return DEFAULT_WIDTH;
  }
}

class HistoryWidth {
  px = $state(stored());
  /** While dragging, Edit turns off its column transition so the edge follows the pointer. */
  dragging = $state(false);

  set(px: number) {
    this.px = clampWidth(px);
  }

  save() {
    try {
      localStorage.setItem(KEY, String(this.px));
    } catch {
      /* a per-viewer convenience only */
    }
  }
}

export const historyWidth = new HistoryWidth();
