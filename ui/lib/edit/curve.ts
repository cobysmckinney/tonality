// Monotone cubic interpolation, mirroring tonality_core::curve (used here only for drawing the
// curve editor; the image itself is rendered from the LUT that Rust computes).
export type Point = [number, number];

export const IDENTITY: Point[] = [
  [0, 0],
  [1, 1],
];

export function normalise(points: Point[]): Point[] {
  const p = points
    .filter(([x, y]) => Number.isFinite(x) && Number.isFinite(y))
    .map(([x, y]): Point => [Math.min(1, Math.max(0, x)), Math.min(1, Math.max(0, y))])
    .sort((a, b) => a[0] - b[0]);
  const out: Point[] = [];
  for (const q of p) if (!out.length || Math.abs(q[0] - out[out.length - 1][0]) >= 1e-4) out.push(q);
  return out.length < 2 ? IDENTITY.map((q) => [...q] as Point) : out;
}

export function isIdentity(points: Point[]): boolean {
  const p = normalise(points);
  return p.length === 2 && p[0][0] === 0 && p[0][1] === 0 && p[1][0] === 1 && p[1][1] === 1;
}

/** Returns an evaluator for the curve through `points`. */
export function curve(points: Point[]): (x: number) => number {
  const p = normalise(points);
  const n = p.length;
  const d = Array.from({ length: n - 1 }, (_, i) => (p[i + 1][1] - p[i][1]) / (p[i + 1][0] - p[i][0]));
  const m = new Array<number>(n).fill(0);
  m[0] = d[0];
  m[n - 1] = d[n - 2];
  for (let i = 1; i < n - 1; i++) m[i] = d[i - 1] * d[i] <= 0 ? 0 : (d[i - 1] + d[i]) / 2;
  for (let i = 0; i < n - 1; i++) {
    if (d[i] === 0) {
      m[i] = m[i + 1] = 0;
      continue;
    }
    const a = m[i] / d[i];
    const b = m[i + 1] / d[i];
    const s = a * a + b * b;
    if (s > 9) {
      const t = 3 / Math.sqrt(s);
      m[i] = t * a * d[i];
      m[i + 1] = t * b * d[i];
    }
  }
  return (x0: number) => {
    const x = Math.min(1, Math.max(0, x0));
    if (x <= p[0][0]) return p[0][1];
    if (x >= p[n - 1][0]) return p[n - 1][1];
    let i = 0;
    while (i < n - 2 && p[i + 1][0] <= x) i++;
    const h = p[i + 1][0] - p[i][0];
    const t = (x - p[i][0]) / h;
    const t2 = t * t;
    const t3 = t2 * t;
    const y =
      (2 * t3 - 3 * t2 + 1) * p[i][1] +
      (t3 - 2 * t2 + t) * h * m[i] +
      (-2 * t3 + 3 * t2) * p[i + 1][1] +
      (t3 - t2) * h * m[i + 1];
    return Math.min(1, Math.max(0, y));
  };
}
