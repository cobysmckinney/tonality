import { describe, expect, it } from "vitest";
import type { ColorGrading } from "$lib/bindings/ColorGrading";
import {
  anchorFor,
  flipped,
  fromPoint,
  GRADING_DEFAULT,
  heldAtCentre,
  linked,
  moved,
  toPoint,
} from "./grading";

/** Hue and saturation of each range, as `[shadows, midtones, highlights]`. */
const dots = (g: ColorGrading) =>
  [g.shadows, g.midtones, g.highlights].map((r) => [r.hue, r.saturation]);

const start = (patch: Partial<ColorGrading> = {}): ColorGrading => ({
  ...structuredClone(GRADING_DEFAULT),
  ...patch,
});
const at = (hue: number, saturation: number, luminance = 0) => ({ hue, saturation, luminance });

describe("colour grading, unlinked", () => {
  it("moves only the dot that was dragged", () => {
    const g = moved(start({ shadows: at(220, 40) }), "highlights", 50, 30);
    expect(dots(g)).toEqual([
      [220, 40],
      [0, 0],
      [50, 30],
    ]);
  });

  it("keeps hue within the circle and saturation within the wheel", () => {
    expect(dots(moved(start(), "shadows", -30.4, 140))[0]).toEqual([330, 100]);
    expect(dots(moved(start(), "shadows", 725, -5))[0]).toEqual([5, 0]);
  });

  it("leaves luminance, blending and balance alone", () => {
    const g = moved(
      start({ shadows: at(0, 0, 25), blending: 80, balance: -10 }),
      "shadows",
      90,
      50,
    );
    expect(g.shadows.luminance).toBe(25);
    expect([g.blending, g.balance]).toEqual([80, -10]);
  });
});

describe("colour grading, linking", () => {
  it("builds each shape around the dot that holds still", () => {
    const g = start({ shadows: at(220, 40) });
    expect(dots(linked(g, "triadic", "shadows"))).toEqual([
      [220, 40],
      [340, 40],
      [100, 40],
    ]);
    expect(dots(linked(g, "analogous", "shadows"))).toEqual([
      [220, 40],
      [250, 40],
      [280, 40],
    ]);
    expect(dots(linked(g, "splitComplementary", "shadows"))).toEqual([
      [220, 40],
      [10, 40],
      [70, 40],
    ]);
  });

  it("puts the midtones at the centre of a line", () => {
    const g = linked(
      start({ shadows: at(220, 40), midtones: at(90, 25) }),
      "complementary",
      "shadows",
    );
    expect(dots(g)).toEqual([
      [220, 40],
      [90, 0],
      [40, 40],
    ]);
    expect(heldAtCentre(g, "midtones")).toBe(true);
    expect(heldAtCentre(g, "shadows")).toBe(false);
    expect(heldAtCentre(start(), "midtones")).toBe(false);
  });

  it("remembers the shape, and forgets it when unlinked without moving anything", () => {
    const on = linked(start({ shadows: at(220, 40) }), "triadic", "shadows");
    expect(on.link).toBe("triadic");
    const off = linked(on, null, "shadows");
    expect(off.link).toBeNull();
    expect(dots(off)).toEqual(dots(on));
  });

  it("takes the way round that moves the other dots least", () => {
    // Highlights already near 100°: the shape goes round the way that puts them there.
    const near = start({ shadows: at(220, 40), highlights: at(110, 20) });
    expect(dots(linked(near, "triadic", "shadows"))[2]).toEqual([100, 40]);
    // Highlights near 340°: it goes round the other way.
    const far = start({ shadows: at(220, 40), highlights: at(330, 20) });
    expect(dots(linked(far, "triadic", "shadows"))).toEqual([
      [220, 40],
      [100, 40],
      [340, 40],
    ]);
  });

  it("builds around a dot that has colour, so linking never wipes the grade", () => {
    const g = start({ shadows: at(220, 40) });
    // Highlights are selected but still at the centre: shadows hold still instead.
    expect(anchorFor(g, "triadic", "highlights")).toBe("shadows");
    expect(dots(linked(g, "triadic", "highlights"))[0]).toEqual([220, 40]);
    // On a line the midtones can't hold still: they are going to the centre.
    const m = start({ midtones: at(90, 25), highlights: at(40, 10) });
    expect(anchorFor(m, "complementary", "midtones")).toBe("highlights");
    expect(anchorFor(m, "triadic", "midtones")).toBe("midtones");
    // Nothing has colour yet: the selected dot will lead.
    expect(anchorFor(start(), "triadic", "highlights")).toBe("highlights");
    expect(anchorFor(start(), "complementary", "midtones")).toBe("shadows");
  });
});

describe("colour grading, linked", () => {
  const triad = linked(start({ shadows: at(220, 40) }), "triadic", "shadows");

  it("turns the whole shape when one dot goes round", () => {
    expect(dots(moved(triad, "highlights", 130, 40))).toEqual([
      [250, 40],
      [10, 40],
      [130, 40],
    ]);
  });

  it("grows and shrinks the whole shape when one dot moves in or out", () => {
    expect(dots(moved(triad, "midtones", 340, 75))).toEqual([
      [220, 75],
      [340, 75],
      [100, 75],
    ]);
  });

  it("starts from the centre: dragging any dot out brings the others with it", () => {
    const g = moved(linked(start(), "splitComplementary", "highlights"), "highlights", 60, 30);
    expect(dots(g)).toEqual([
      [210, 30],
      [0, 30],
      [60, 30],
    ]);
  });

  it("keeps the way round it was built", () => {
    const far = linked(
      start({ shadows: at(220, 40), highlights: at(330, 20) }),
      "triadic",
      "shadows",
    );
    expect(dots(moved(far, "shadows", 230, 40))).toEqual([
      [230, 40],
      [110, 40],
      [350, 40],
    ]);
  });

  it("holds the midtones at the centre of a line while the ends move", () => {
    const line = linked(start({ shadows: at(220, 40) }), "complementary", "shadows");
    expect(dots(moved(line, "highlights", 60, 55))).toEqual([
      [240, 55],
      [0, 0],
      [60, 55],
    ]);
    expect(moved(line, "midtones", 120, 80)).toEqual(line);
  });

  it("flips to the mirror image around the dot that holds still", () => {
    expect(dots(flipped(triad, "shadows"))).toEqual([
      [220, 40],
      [100, 40],
      [340, 40],
    ]);
    const neighbours = linked(start({ midtones: at(100, 50) }), "analogous", "midtones");
    expect(dots(neighbours)).toEqual([
      [70, 50],
      [100, 50],
      [130, 50],
    ]);
    expect(dots(flipped(neighbours, "midtones"))).toEqual([
      [130, 50],
      [100, 50],
      [70, 50],
    ]);
    // Unlinked, or a line (its own mirror image): nothing to flip.
    expect(flipped(start({ shadows: at(220, 40) }), "shadows")).toEqual(
      start({ shadows: at(220, 40) }),
    );
  });

  it("switches shape around the same dot, going round the same way", () => {
    expect(dots(linked(triad, "analogous", "shadows"))).toEqual([
      [220, 40],
      [250, 40],
      [280, 40],
    ]);
    expect(dots(linked(flipped(triad, "shadows"), "analogous", "shadows"))).toEqual([
      [220, 40],
      [190, 40],
      [160, 40],
    ]);
  });
});

describe("colour grading, the wheel", () => {
  it("puts hue 0 to the right and counts anticlockwise, y up", () => {
    const p = toPoint(at(90, 50));
    expect(p.x).toBeCloseTo(0);
    expect(p.y).toBeCloseTo(0.5);
    expect(fromPoint(0.5, 0)).toEqual({ hue: 0, saturation: 50 });
    expect(fromPoint(0, -0.25)).toEqual({ hue: 270, saturation: 25 });
  });

  it("stops at the rim", () => {
    expect(fromPoint(-3, 0)).toEqual({ hue: 180, saturation: 100 });
  });
});
