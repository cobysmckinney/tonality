import { describe, expect, it } from "vitest";
import { paint } from "./brush";

describe("brush", () => {
  // Same stroke and numbers as `reference_values_for_the_preview` in tonality-core's mask.rs.
  it("paints exactly what the Rust rasteriser does", () => {
    const [w, h] = [64, 48];
    const layer = new Uint8Array(w * h);
    paint(layer, w, h, {
      points: [
        [0.25, 0.5],
        [0.75, 0.25],
      ],
      radius: 0.1,
      feather: 0.7,
      strength: 0.8,
      erase: false,
    });
    const sum = layer.reduce((a, v) => a + v, 0);
    const samples = [
      [16, 24],
      [16, 27],
      [20, 26],
      [32, 21],
      [40, 19],
      [16, 30],
      [50, 9],
      [11, 24],
    ].map(([x, y]) => layer[y * w + x]);
    expect([sum, samples]).toEqual([69740, [204, 144, 118, 149, 86, 0, 144, 77]]);
  });

  it("erases", () => {
    const layer = new Uint8Array(100 * 100).fill(255);
    paint(layer, 100, 100, {
      points: [[0.5, 0.5]],
      radius: 0.1,
      feather: 0,
      strength: 1,
      erase: true,
    });
    expect(layer[50 * 100 + 50]).toBe(0);
    expect(layer[0]).toBe(255);
  });
});
