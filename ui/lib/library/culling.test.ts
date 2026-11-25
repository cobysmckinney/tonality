import { describe, expect, it } from "vitest";
import { flagNote, nextRating, ratingNote, sameJson } from "./culling";

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

describe("nextRating", () => {
  it("sets the pressed rating", () => {
    expect(nextRating([0], 3)).toBe(3);
    expect(nextRating([3, 2], 3)).toBe(3);
  });
  it("clears it when every photo already has it", () => {
    expect(nextRating([3], 3)).toBe(0);
    expect(nextRating([3, 3], 3)).toBe(0);
  });
  it("sets it for a photo it can't see", () => {
    expect(nextRating([undefined], 3)).toBe(3);
    expect(nextRating([], 3)).toBe(3);
  });
});

describe("notes", () => {
  it("say what changed", () => {
    expect(ratingNote(3)).toBe("★★★");
    expect(ratingNote(0)).toBe("No rating");
    expect(flagNote(1)).toBe("Picked");
    expect(flagNote(-1)).toBe("Rejected");
    expect(flagNote(0)).toBe("Unflagged");
  });
});
