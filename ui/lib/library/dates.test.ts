import { describe, expect, it } from "vitest";
import { dayKey, dayLabel } from "./dates";

describe("capture days", () => {
  it("keys by local calendar day", () => {
    expect(dayKey("2026-11-02T14:03:11")).toBe("2026-11-02");
    expect(dayKey(null)).toBe("");
    expect(dayKey("garbage")).toBe("");
  });

  it("labels recent, this-year and older days", () => {
    const now = new Date(2026, 10, 10, 15);
    expect(dayLabel("2026-11-10", now)).toBe("Today");
    expect(dayLabel("2026-11-09", now)).toBe("Yesterday");
    expect(dayLabel("2026-11-02", now)).toContain("November");
    expect(dayLabel("2025-11-02", now)).toContain("2025");
    expect(dayLabel("", now)).toBe("No date");
  });
});
