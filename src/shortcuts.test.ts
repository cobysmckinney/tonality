import { describe, expect, test } from "bun:test";
import { editorKeysBlocked } from "./shortcuts";

const editing = { confirmRequest: null, menu: null, shortcutsOpen: false, aboutOpen: false, importState: null, exportState: null, filmSheet: null, openId: 7 };

describe("the editor's keys", () => {
  test("work with a photo open and nothing over it", () => {
    expect(editorKeysBlocked(editing)).toBe(false);
  });

  test("wait for a menu, a question, the list of shortcuts, the About screen, or a sheet", () => {
    expect(editorKeysBlocked({ ...editing, menu: { x: 0, y: 0, entries: [] } })).toBe(true);
    expect(editorKeysBlocked({ ...editing, shortcutsOpen: true })).toBe(true);
    expect(editorKeysBlocked({ ...editing, aboutOpen: true })).toBe(true);
    expect(editorKeysBlocked({ ...editing, confirmRequest: { title: "Delete?" } })).toBe(true);
    expect(editorKeysBlocked({ ...editing, importState: { phase: "review" } })).toBe(true);
    expect(editorKeysBlocked({ ...editing, exportState: { phase: "setup" } })).toBe(true);
    expect(editorKeysBlocked({ ...editing, filmSheet: { photos: [] } })).toBe(true);
  });

  test("do nothing without a photo open", () => {
    expect(editorKeysBlocked({ ...editing, openId: null })).toBe(true);
  });
});
