import { describe, expect, it } from "vitest";
import { aperture, shootingInfo, shutter } from "./shooting";

describe("shutter", () => {
  it("names fast speeds as fractions", () => {
    expect(shutter(1 / 250)).toBe("1/250 s");
    expect(shutter(0.004)).toBe("1/250 s");
    expect(shutter(1 / 8000)).toBe("1/8000 s");
    expect(shutter(0.25)).toBe("1/4 s");
  });
  it("names slow speeds in seconds", () => {
    expect(shutter(0.6)).toBe("0.6 s");
    expect(shutter(1)).toBe("1 s");
    expect(shutter(2.5)).toBe("2.5 s");
    expect(shutter(30)).toBe("30 s");
  });
});

describe("aperture", () => {
  it("keeps one decimal under f/10", () => {
    expect(aperture(2.8)).toBe("f/2.8");
    expect(aperture(8)).toBe("f/8");
    expect(aperture(1.4142)).toBe("f/1.4");
    expect(aperture(11)).toBe("f/11");
    expect(aperture(22.6)).toBe("f/23");
  });
});

describe("shootingInfo", () => {
  it("lists the four settings", () => {
    expect(shootingInfo({ iso: 100, exposureTime: 0.004, fNumber: 2.8, focalLength: 35 })).toEqual([
      "ISO 100",
      "1/250 s",
      "f/2.8",
      "35 mm",
    ]);
  });
  it("leaves out what's missing or meaningless", () => {
    expect(shootingInfo({ iso: null, exposureTime: 0.5, fNumber: 0, focalLength: 24.4 })).toEqual([
      "0.5 s",
      "24 mm",
    ]);
    expect(
      shootingInfo({ iso: null, exposureTime: null, fNumber: null, focalLength: null }),
    ).toEqual([]);
    expect(
      shootingInfo({ iso: NaN, exposureTime: -1, fNumber: Infinity, focalLength: null }),
    ).toEqual([]);
  });
});
