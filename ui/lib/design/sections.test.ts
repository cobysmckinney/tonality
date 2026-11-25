import { beforeEach, describe, expect, it, vi } from "vitest";

const store = new Map<string, string>();
vi.stubGlobal("localStorage", {
  getItem: (k: string) => store.get(k) ?? null,
  setItem: (k: string, v: string) => void store.set(k, v),
});

const { rememberOpen, sections, wasOpen } = await import("./sections.svelte");

beforeEach(() => store.clear());

describe("section open state", () => {
  it("uses the section's default until it's toggled", () => {
    expect(wasOpen("Light", true)).toBe(true);
    expect(wasOpen("Detail", false)).toBe(false);
  });
  it("remembers open and closed per section", () => {
    rememberOpen("Light", false);
    rememberOpen("Detail", true);
    expect(wasOpen("Light", true)).toBe(false);
    expect(wasOpen("Detail", false)).toBe(true);
    // Kept under the same keys as before, so what people had open survives the update.
    expect(store.get("tonality.section.Light")).toBe("0");
  });
});

describe("one section at a time", () => {
  it("is off by default and remembered", () => {
    expect(sections.oneAtATime).toBe(false);
    sections.setOneAtATime(true);
    expect(sections.oneAtATime).toBe(true);
    expect(store.get("tonality.sections.oneAtATime")).toBe("1");
    sections.setOneAtATime(false);
    expect(store.get("tonality.sections.oneAtATime")).toBe("0");
  });
  it("counts each opening, even of the same section", () => {
    sections.open("Light");
    const a = sections.opened!;
    sections.open("Light");
    const b = sections.opened!;
    expect(a.key).toBe("Light");
    expect(b.n).toBeGreaterThan(a.n);
  });
});
