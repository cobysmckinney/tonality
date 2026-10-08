import { expect, test } from "bun:test";
import { aperture } from "./format";

test("apertures show as the camera wrote them, without trailing zeros", () => {
  expect(aperture(0.95)).toBe("ƒ/0.95");
  expect(aperture(1.2)).toBe("ƒ/1.2");
  expect(aperture(2.8)).toBe("ƒ/2.8");
  expect(aperture(8)).toBe("ƒ/8");
  expect(aperture(5.599999)).toBe("ƒ/5.6");
});
