// The Merge dialog's panels, and the choice of them it remembers between uses.
import type { Panel } from "$lib/bindings/Panel";

export const MERGE_PANELS: { id: Panel; label: string }[] = [
  { id: "whiteBalance", label: "White Balance" },
  { id: "light", label: "Light" },
  { id: "toneCurve", label: "Tone Curve" },
  { id: "color", label: "Color" },
  { id: "colorGrading", label: "Color Grading" },
  { id: "detail", label: "Detail" },
  { id: "effects", label: "Effects" },
  { id: "crop", label: "Crop" },
  { id: "masks", label: "Masks" },
  { id: "lens", label: "Lens Correction" },
];

const DEFAULT: Panel[] = ["color"];

/** A remembered choice (JSON), keeping only panels that still exist; the default if none. */
export function parsePanels(raw: string | null): Set<Panel> {
  try {
    const known = new Set(MERGE_PANELS.map((p) => p.id));
    const list = JSON.parse(raw ?? "null");
    if (Array.isArray(list)) {
      const ok = list.filter((p): p is Panel => known.has(p));
      if (ok.length) return new Set(ok);
    }
  } catch {
    /* fall through to the default */
  }
  return new Set(DEFAULT);
}

/** The chosen panels in the dialog's order. */
export const orderedPanels = (chosen: Set<Panel>): Panel[] =>
  MERGE_PANELS.map((p) => p.id).filter((p) => chosen.has(p));
