import { describe, expect, it } from "vitest";
import type { EditState } from "$lib/bindings/EditState";
import type { Mask } from "$lib/bindings/Mask";
import { aiKey, brushKey, brushLayer, newAiMask, nextName } from "./masks";

const brush = (): Mask => ({ ...newAiMask("subject", { kind: "subject" }, "subject@1", []), name: "Brush 1", shape: { kind: "brush", strokes: [] } });

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
    const edit = { masks: [newAiMask("sky", { kind: "sky" }, "scene@1", [])] } as unknown as EditState;
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
