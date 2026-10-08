import { describe, expect, test } from "bun:test";
import { forget, Pointers } from "./selection";

const visible = [1, 2, 3, 4, 5, 6];

const on = (id: number, selection: number[] = [id]): Pointers => ({
  selection: new Set(selection),
  anchor: id,
  cursor: id,
  openId: null,
});

describe("photos leaving the grid", () => {
  test("the selection moves to the next photo, else the previous one", () => {
    expect(forget(visible, new Set([3]), on(3))).toEqual({ ...on(4), anchor: 4 });
    expect(forget(visible, new Set([6]), on(6))).toEqual(on(5));
  });

  test("a run of photos steps past the whole run", () => {
    expect(forget(visible, new Set([3, 4, 5]), on(5, [3, 4, 5])).cursor).toBe(6);
    expect(forget(visible, new Set([3, 4, 5]), on(3, [3, 4, 5])).cursor).toBe(6);
  });

  test("when nothing is left, nothing is selected", () => {
    expect(forget(visible, new Set(visible), on(2))).toEqual({
      selection: new Set(),
      anchor: null,
      cursor: null,
      openId: null,
    });
  });

  test("the rest of a selection stays as it was", () => {
    const after = forget([1, 2, 3, 4], new Set([2]), { ...on(2, [1, 2, 3]), anchor: 1 });
    expect(after.selection).toEqual(new Set([1, 3]));
    expect(after.anchor).toBe(1);
    expect(after.cursor).toBe(3);
  });

  test("a range start that left is dropped", () => {
    const after = forget([1, 2, 3, 4], new Set([1]), { ...on(3, [1, 2, 3]), anchor: 1 });
    expect(after.anchor).toBeNull();
    expect(after.cursor).toBe(3);
  });

  test("an unselected cursor moves on without selecting anything", () => {
    const after = forget(visible, new Set([2]), { ...on(2, []), anchor: null });
    expect(after.selection).toEqual(new Set());
    expect(after.cursor).toBe(3);
  });

  test("photos that stay are left alone", () => {
    const before = { ...on(4, [4, 5]), openId: null };
    expect(forget(visible, new Set([1]), before)).toEqual(before);
  });

  test("the open photo steps to its neighbour, and closes when none is left", () => {
    const open = { ...on(3), openId: 3 };
    expect(forget(visible, new Set([3]), open)).toEqual({ ...on(4), openId: 4 });
    expect(forget(visible, new Set([6]), { ...on(6), openId: 6 }).openId).toBe(5);
    expect(forget([3], new Set([3]), open).openId).toBeNull();
  });
});
