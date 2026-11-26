import { describe, expect, it } from "vitest";
import type { EditState } from "$lib/bindings/EditState";
import type { Mask } from "$lib/bindings/Mask";
import {
  aiKey,
  brushKey,
  brushLayer,
  maskSize,
  newAiMask,
  nextName,
  shaderIndex,
  strokesHash,
  withoutStrokes,
} from "./masks";

const brush = (): Mask => ({
  ...newAiMask("subject", { kind: "subject" }, "subject@1", []),
  name: "Brush 1",
  shape: { kind: "brush", strokes: [] },
});

describe("AI masks", () => {
  it("names presets and inverts Background", () => {
    const sky = newAiMask("sky", { kind: "sky" }, "scene@1", []);
    expect(sky.name).toBe("Sky 1");
    expect(nextName([sky], "sky")).toBe("Sky 2");
    const bg = newAiMask("background", { kind: "subject" }, "subject@1", []);
    expect(bg.name).toBe("Background 1");
    expect(bg.invert).toBe(true);
    expect(sky.invert).toBe(false);
  });

  it("counts AI masks as raster layers", () => {
    const masks = [brush(), newAiMask("sky", { kind: "sky" }, "scene@1", []), brush()];
    expect(brushLayer(masks, 2)).toBe(2);
  });

  it("refetches layers after a detection or a lens change, not otherwise", () => {
    const edit = {
      masks: [newAiMask("sky", { kind: "sky" }, "scene@1", [])],
    } as unknown as EditState;
    const a = brushKey(edit, 0);
    expect(a).not.toBe("");
    expect(brushKey(edit, 0)).toBe(a);
    expect(brushKey(edit, 1)).not.toBe(a);
    expect(brushKey({ ...edit, lens: { profile: true } } as unknown as EditState, 0)).not.toBe(a);
    // Brush-only states don't depend on the epoch.
    const b = { masks: [brush()] } as unknown as EditState;
    expect(brushKey(b, 0)).toBe(brushKey(b, 5));
  });

  it("keys detections by model and target", () => {
    const o = newAiMask("object", { kind: "object", lasso: [[0.1, 0.2]] }, "sam@1", []);
    expect(aiKey(o)).toBe('sam@1:{"kind":"object","lasso":[[0.1,0.2]]}');
    expect(aiKey(brush())).toBeNull();
  });
});

describe("stroke keys", () => {
  const st = (x: number, erase = false) => ({
    points: [[x, 0.5]] as [number, number][],
    radius: 0.05,
    feather: 0.5,
    strength: 1,
    erase,
  });
  it("change with any stroke detail and stay put otherwise", () => {
    const a = strokesHash([st(0.1), st(0.2)]);
    expect(strokesHash([st(0.1), st(0.2)])).toBe(a);
    expect(strokesHash([st(0.1), st(0.3)])).not.toBe(a);
    expect(strokesHash([st(0.1), st(0.2, true)])).not.toBe(a);
    expect(strokesHash([st(0.1)])).not.toBe(a);
  });
  it("brush keys follow the strokes", () => {
    const m = brush();
    const k0 = brushKey({ masks: [m] } as unknown as EditState);
    if (m.shape.kind === "brush") m.shape.strokes.push(st(0.4));
    expect(brushKey({ masks: [m] } as unknown as EditState)).not.toBe(k0);
  });
  it("withoutStrokes drops points only", () => {
    const m = brush();
    if (m.shape.kind === "brush") m.shape.strokes.push(st(0.4));
    const s = withoutStrokes({ masks: [m] } as unknown as EditState);
    expect(s.masks![0].shape).toEqual({ kind: "brush", strokes: [] });
    expect(s.masks![0].name).toBe(m.name);
    expect(m.shape.kind === "brush" && m.shape.strokes.length).toBe(1);
  });
});

describe("the mask raster's size", () => {
  // `[full width, full height, raster width, raster height]`. The same table is in
  // `crates/tonality-core/src/mask.rs` (`mask_size_matches_the_interface`), with the numbers
  // its `mask_size` gives: the two have to agree exactly, or the rasters painted live in the
  // preview and the ones the backend sends are different sizes.
  const SIZES = [
    [6000, 4000, 1024, 683],
    [4000, 6000, 683, 1024],
    [7952, 5304, 1024, 683],
    [5304, 7952, 683, 1024],
    [8256, 5504, 1024, 683],
    [9504, 6336, 1024, 683],
    [6240, 4160, 1024, 683],
    [4672, 7008, 683, 1024],
    [1000, 656, 1024, 672],
    [1, 1, 1024, 1024],
    [3, 2, 1024, 683],
    [2000, 1312, 1024, 672],
    [6016, 4016, 1024, 684],
    [11648, 8736, 1024, 768],
    [3000, 2001, 1024, 683],
    [5472, 3648, 1024, 683],
    [1023, 683, 1024, 684],
    [1025, 7, 1024, 7],
  ];
  it("is what the backend computes", () => {
    for (const [w, h, mw, mh] of SIZES) expect([w, h, ...maskSize(w, h)]).toEqual([w, h, mw, mh]);
  });
});

describe("shaderIndex", () => {
  it("counts only the masks that are drawn", () => {
    const masks = [brush(), { ...brush(), enabled: false }, brush()];
    expect(masks.map((_, i) => shaderIndex(masks, i))).toEqual([0, null, 1]);
    expect(shaderIndex(masks, 7)).toBeNull();
  });
});
