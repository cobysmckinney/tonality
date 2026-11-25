import { describe, expect, it } from "vitest";
import { sameJson } from "./culling";

describe("sameJson", () => {
  const state = { light: { exposure: 0, contrast: 0 }, masks: [], crop: null };

  it("matches equal states however they were built", () => {
    expect(sameJson(state, structuredClone(state))).toBe(true);
    expect(sameJson({ a: 1, b: 2 }, { b: 2, a: 1 })).toBe(true);
  });

  it("sees a changed, missing or extra value", () => {
    expect(sameJson(state, { ...state, light: { exposure: 0.3, contrast: 0 } })).toBe(false);
    expect(sameJson(state, { light: state.light, masks: [] })).toBe(false);
    expect(sameJson(state, { ...state, masks: [{ id: 1 }] })).toBe(false);
    expect(sameJson({ a: null }, { a: 0 })).toBe(false);
    expect(sameJson({ a: undefined }, { b: undefined })).toBe(false);
    expect(sameJson([], {})).toBe(false);
  });
});
