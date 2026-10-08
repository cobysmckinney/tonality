import { expect, test } from "bun:test";
import { aperture, shutter } from "./format";

test("apertures show as the camera wrote them, without trailing zeros", () => {
  expect(aperture(0.95)).toBe("ƒ/0.95");
  expect(aperture(1.2)).toBe("ƒ/1.2");
  expect(aperture(2.8)).toBe("ƒ/2.8");
  expect(aperture(8)).toBe("ƒ/8");
  expect(aperture(5.599999)).toBe("ƒ/5.6");
});

test("shutter speeds read as the camera wrote them", () => {
  const cases: [number, string][] = [
    [1 / 250, "1/250s"],
    [1 / 4, "1/4s"],
    [1 / 3, "1/3s"],
    [0.3, "0.3s"],
    [0.4, "0.4s"],
    [0.5, "1/2s"],
    [0.8, "0.8s"],
    [1, "1s"],
    [1.3, "1.3s"],
    [30, "30s"],
  ];
  for (const [seconds, shown] of cases) expect(shutter(seconds)).toBe(shown);
});
