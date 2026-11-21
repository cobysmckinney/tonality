import { describe, expect, it } from "vitest";
import { enterConfirms, targetOwnsKey } from "./keys";

// No DOM in the test environment: a minimal element answering `closest`/`matches` for a
// comma-separated selector by tag name or by the selectors listed in `is`.
class Fake {
  constructor(
    public tagName: string,
    public opts: { type?: string; is?: string[]; parent?: Fake } = {},
  ) {}
  get type() {
    return this.opts.type;
  }
  matches(sel: string): boolean {
    return sel.split(/,(?![^[]*\])/).some((s) => {
      const t = s.trim();
      return t === this.tagName.toLowerCase() || (this.opts.is ?? []).includes(t);
    });
  }
  closest(sel: string): Fake | null {
    return this.matches(sel) ? this : (this.opts.parent?.closest(sel) ?? null);
  }
}
const el = (tag: string, opts: Fake["opts"] = {}) => new Fake(tag, opts) as unknown as EventTarget;
const owns = (t: EventTarget | null, ...keys: string[]) => keys.map((k) => targetOwnsKey(t, k));

describe("targetOwnsKey", () => {
  it("buttons and links take Enter and Space, not letters or arrows", () => {
    for (const t of [
      el("BUTTON"),
      el("A", { is: ["a[href]"] }),
      el("DIV", { is: ["[role=button]"] }),
    ])
      expect(owns(t, "Enter", " ", "e", "ArrowDown")).toEqual([true, true, false, false]);
  });

  it("counts the svg inside a button as the button", () => {
    const icon = el("svg", { parent: new Fake("BUTTON") });
    expect(targetOwnsKey(icon, "Enter")).toBe(true);
  });

  it("text fields and selects take every key", () => {
    for (const t of [el("INPUT"), el("INPUT", { type: "text" }), el("TEXTAREA"), el("SELECT")])
      expect(owns(t, "Enter", "e", "g", "ArrowLeft", "1")).toEqual([true, true, true, true, true]);
  });

  it("sliders, radios and menu items take arrows", () => {
    expect(owns(el("INPUT", { type: "range" }), "ArrowLeft", "Home", "Enter", "e")).toEqual([
      true,
      true,
      false,
      false,
    ]);
    expect(owns(el("DIV", { is: ["[role=slider]"] }), "ArrowUp", "Enter")).toEqual([true, false]);
    expect(owns(el("BUTTON", { is: ["[role=menuitem]"] }), "ArrowDown", "Enter")).toEqual([
      true,
      true,
    ]);
  });

  it("a checkbox takes only Space", () => {
    expect(owns(el("INPUT", { type: "checkbox" }), " ", "Enter", "e")).toEqual([
      true,
      false,
      false,
    ]);
  });

  it("an element marked data-shortcuts leaves its keys to the window", () => {
    const cell = el("DIV", { is: ["[role=button]", "[data-shortcuts]"] });
    expect(owns(cell, "Enter", "ArrowRight", "e")).toEqual([false, false, false]);
  });

  it("the page, a plain element or no target own nothing", () => {
    expect(targetOwnsKey(el("BODY"), "Enter")).toBe(false);
    expect(targetOwnsKey(el("DIV", { parent: new Fake("MAIN") }), "Enter")).toBe(false);
    expect(targetOwnsKey(null, "Enter")).toBe(false);
    expect(targetOwnsKey(window_(), "Enter")).toBe(false);
  });
});

describe("enterConfirms", () => {
  const key = (target: EventTarget, k = "Enter") => ({ key: k, target }) as KeyboardEvent;
  it("only Enter, and not on a focused button or field", () => {
    expect(enterConfirms(key(el("DIV", { is: ["[role=dialog]"] })))).toBe(true);
    expect(enterConfirms(key(el("INPUT", { type: "checkbox" })))).toBe(true);
    expect(enterConfirms(key(el("BUTTON")))).toBe(false);
    expect(enterConfirms(key(el("INPUT", { type: "text" })))).toBe(false);
    expect(enterConfirms(key(el("DIV"), "Escape"))).toBe(false);
  });
});

/** Something that isn't an element (the window). */
function window_(): EventTarget {
  return {} as EventTarget;
}
