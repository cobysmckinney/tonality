import { expect, test } from "bun:test";
import { shutter } from "./format";

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
