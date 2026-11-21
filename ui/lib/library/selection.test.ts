import { describe, expect, it } from "vitest";
import { arrowSelect, clickSelect, rangeIds, type Selection } from "./selection";

// A 4-wide grid: Down is +4, Right +1.
const ids = Array.from({ length: 12 }, (_, i) => i + 1);
const fresh = (): Selection => ({ selected: new Set(), focus: null, anchor: null });
const sel = (s: Selection) => [...s.selected].sort((a, b) => a - b);

describe("rangeIds", () => {
  it("spans either direction, inclusive", () => {
    expect(rangeIds(ids, 3, 6)).toEqual([3, 4, 5, 6]);
    expect(rangeIds(ids, 6, 3)).toEqual([3, 4, 5, 6]);
    expect(rangeIds(ids, 5, 5)).toEqual([5]);
    expect(rangeIds(ids, 5, 99)).toBeNull();
  });
});

describe("Shift+Arrow", () => {
  it("Shift+Down selects every photo between, not just the one below", () => {
    const s = fresh();
    clickSelect(s, ids, 2);
    arrowSelect(s, ids, 6, true);
    expect(sel(s)).toEqual([2, 3, 4, 5, 6]);
    expect(s.focus).toBe(6);
    expect(s.anchor).toBe(2);
  });

  it("Shift+Right then Shift+Left shrinks back", () => {
    const s = fresh();
    clickSelect(s, ids, 5);
    arrowSelect(s, ids, 6, true);
    arrowSelect(s, ids, 7, true);
    expect(sel(s)).toEqual([5, 6, 7]);
    arrowSelect(s, ids, 6, true);
    expect(sel(s)).toEqual([5, 6]);
    arrowSelect(s, ids, 5, true);
    expect(sel(s)).toEqual([5]);
    arrowSelect(s, ids, 4, true);
    expect(sel(s)).toEqual([4, 5]);
  });

  it("a later Shift-click measures from where the selection started", () => {
    const s = fresh();
    clickSelect(s, ids, 5);
    arrowSelect(s, ids, 9, true);
    clickSelect(s, ids, 3, { shiftKey: true });
    expect(sel(s)).toEqual([3, 4, 5]);
    expect(s.anchor).toBe(5);
    expect(s.focus).toBe(3);
  });

  it("a plain arrow moves the anchor", () => {
    const s = fresh();
    clickSelect(s, ids, 5);
    arrowSelect(s, ids, 6, false);
    expect(sel(s)).toEqual([6]);
    expect(s.anchor).toBe(6);
    arrowSelect(s, ids, 8, true);
    expect(sel(s)).toEqual([6, 7, 8]);
  });

  it("starts from the focus when nothing set an anchor", () => {
    const s: Selection = { selected: new Set([3]), focus: 3, anchor: null };
    arrowSelect(s, ids, 4, true);
    expect(sel(s)).toEqual([3, 4]);
    expect(s.anchor).toBe(3);
  });

  it("with no focus at all, just selects the photo", () => {
    const s = fresh();
    arrowSelect(s, ids, 1, true);
    expect(sel(s)).toEqual([1]);
    expect(s.focus).toBe(1);
  });
});

describe("clicks", () => {
  it("Shift-click replaces, Ctrl+Shift-click adds the range", () => {
    const s = fresh();
    clickSelect(s, ids, 2);
    clickSelect(s, ids, 10, { ctrlKey: true });
    expect(s.anchor).toBe(10);
    clickSelect(s, ids, 12, { shiftKey: true, ctrlKey: true });
    expect(sel(s)).toEqual([2, 10, 11, 12]);
    clickSelect(s, ids, 8, { shiftKey: true });
    expect(sel(s)).toEqual([8, 9, 10]);
  });

  it("Ctrl-click toggles", () => {
    const s = fresh();
    clickSelect(s, ids, 2);
    clickSelect(s, ids, 2, { metaKey: true });
    expect(sel(s)).toEqual([]);
  });
});
