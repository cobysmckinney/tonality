<script lang="ts">
  // Point tone curve: master RGB plus per-channel curves.
  // Click the curve to add a point · drag points · double-click (or drag off the graph) to remove.
  import type { ToneCurve } from "$lib/bindings/ToneCurve";
  import { curve as evaluator, IDENTITY, isIdentity, type Point } from "./curve";
  import type { Histogram } from "./renderer";

  type Channel = keyof ToneCurve;
  interface Props {
    value: ToneCurve;
    histogram: Histogram | null;
    oninput: (c: ToneCurve) => void;
    oncommit: () => void;
  }
  let { value, histogram, oninput, oncommit }: Props = $props();

  const channels: { id: Channel; label: string; colour: string }[] = [
    { id: "rgb", label: "RGB", colour: "var(--text-0)" },
    { id: "red", label: "R", colour: "#e5484d" },
    { id: "green", label: "G", colour: "#46c46a" },
    { id: "blue", label: "B", colour: "#4d82ee" },
  ];
  let channel = $state<Channel>("rgb");
  const colour = $derived(channels.find((c) => c.id === channel)!.colour);

  let size = $state(240);
  let svg: SVGSVGElement;

  const points = $derived<Point[]>(value[channel].length ? (value[channel] as Point[]) : IDENTITY);
  const f = $derived(evaluator(points));
  const path = $derived(
    Array.from({ length: 129 }, (_, i) => {
      const x = i / 128;
      return `${i ? "L" : "M"}${(x * size).toFixed(1)},${((1 - f(x)) * size).toFixed(1)}`;
    }).join(""),
  );
  const histPath = $derived.by(() => {
    const h = histogram;
    if (!h) return "";
    const src =
      channel === "rgb" ? h.y : h[channel === "red" ? "r" : channel === "green" ? "g" : "b"];
    let peak = 1;
    for (let i = 1; i < 255; i++) peak = Math.max(peak, src[i]);
    let d = `M0,${size}`;
    for (let i = 0; i < 256; i++)
      d += `L${((i / 255) * size).toFixed(1)},${(size - Math.min(1, Math.sqrt(src[i] / peak)) * size * 0.9).toFixed(1)}`;
    return d + `L${size},${size}Z`;
  });

  // ---- editing ----
  let drag: { index: number; removed: boolean } | null = null;

  function toUnit(e: PointerEvent): Point {
    const r = svg.getBoundingClientRect();
    return [(e.clientX - r.left) / r.width, 1 - (e.clientY - r.top) / r.height];
  }

  function emit(pts: Point[]) {
    oninput({
      ...value,
      [channel]: isIdentity(pts) ? [] : pts.map(([x, y]) => [round(x), round(y)]),
    });
  }
  const round = (v: number) => Math.round(v * 10000) / 10000;

  function nearest(p: Point): number {
    let best = -1;
    let bestD = (12 / size) ** 2;
    points.forEach(([x, y], i) => {
      const d = (x - p[0]) ** 2 + (y - p[1]) ** 2;
      if (d < bestD) {
        bestD = d;
        best = i;
      }
    });
    return best;
  }

  function onDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const p = toUnit(e);
    let index = nearest(p);
    let pts = points.map((q) => [...q] as Point);
    if (index < 0) {
      // Add a point on the curve at this x.
      const x = Math.min(0.99, Math.max(0.01, p[0]));
      pts.push([x, f(x)]);
      pts.sort((a, b) => a[0] - b[0]);
      index = pts.findIndex((q) => q[0] === x);
      emit(pts);
    }
    drag = { index, removed: false };
    svg.setPointerCapture(e.pointerId);
  }

  function onMove(e: PointerEvent) {
    if (!drag) return;
    const [x, y] = toUnit(e);
    const pts = (value[channel].length ? (value[channel] as Point[]) : IDENTITY).map(
      (q) => [...q] as Point,
    );
    const i = drag.index;
    const last = pts.length - 1;
    const interior = i > 0 && i < last;
    // Dragging an interior point well outside the graph removes it (until dragged back).
    const off = interior && (y < -0.12 || y > 1.12 || x < -0.12 || x > 1.12);
    if (off !== drag.removed) drag.removed = off;
    const lo = i > 0 ? pts[i - 1][0] + 0.01 : 0;
    const hi = i < last ? pts[i + 1][0] - 0.01 : 1;
    pts[i] = [Math.min(hi, Math.max(lo, x)), Math.min(1, Math.max(0, y))];
    emit(off ? pts.filter((_, j) => j !== i) : pts);
  }

  function onUp(e: PointerEvent) {
    if (!drag) return;
    svg.releasePointerCapture(e.pointerId);
    drag = null;
    oncommit();
  }

  function onDouble(e: MouseEvent) {
    const r = svg.getBoundingClientRect();
    const p: Point = [(e.clientX - r.left) / r.width, 1 - (e.clientY - r.top) / r.height];
    const i = nearest(p);
    if (i > 0 && i < points.length - 1) {
      emit(points.filter((_, j) => j !== i));
      oncommit();
    }
  }

  function resetChannel() {
    oninput({ ...value, [channel]: [] });
    oncommit();
  }

  const edited = (c: Channel) => value[c].length > 0 && !isIdentity(value[c] as Point[]);
</script>

<div class="curve">
  <div class="tabs">
    {#each channels as c (c.id)}
      <button class:active={channel === c.id} onclick={() => (channel = c.id)} style:--c={c.colour}>
        {c.label}{#if edited(c.id)}<span class="dot"></span>{/if}
      </button>
    {/each}
    <button
      class="reset"
      onclick={resetChannel}
      disabled={!edited(channel)}
      title="Reset this curve">Reset</button
    >
  </div>

  <div class="graph" bind:clientWidth={size} style:height="{size}px">
    <svg
      bind:this={svg}
      viewBox="0 0 {size} {size}"
      width={size}
      height={size}
      onpointerdown={onDown}
      onpointermove={onMove}
      onpointerup={onUp}
      onpointercancel={onUp}
      ondblclick={onDouble}
      role="application"
      aria-label="Tone curve"
    >
      {#if histPath}<path d={histPath} class="hist" />{/if}
      {#each [0.25, 0.5, 0.75] as g (g)}
        <line x1={g * size} y1="0" x2={g * size} y2={size} class="grid" />
        <line x1="0" y1={g * size} x2={size} y2={g * size} class="grid" />
      {/each}
      <line x1="0" y1={size} x2={size} y2="0" class="diag" />
      <path d={path} class="line" style:stroke={colour} />
      {#each points as [x, y], i (i)}
        <circle cx={x * size} cy={(1 - y) * size} r="4" style:stroke={colour} />
      {/each}
    </svg>
  </div>
</div>

<style>
  .curve {
    display: grid;
    gap: var(--space-2);
  }
  .tabs {
    display: flex;
    gap: 2px;
    align-items: center;
  }
  .tabs button {
    position: relative;
    padding: 2px 8px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--c);
    opacity: 0.55;
  }
  .tabs button.active {
    background: var(--bg-3);
    opacity: 1;
  }
  .tabs button:hover {
    opacity: 1;
  }
  .dot {
    position: absolute;
    top: 2px;
    right: 2px;
    width: 4px;
    height: 4px;
    border-radius: 50%;
    background: currentColor;
  }
  .tabs .reset {
    margin-left: auto;
    color: var(--text-2);
    opacity: 1;
  }
  .tabs .reset:hover:not(:disabled) {
    color: var(--text-0);
  }
  .tabs .reset:disabled {
    opacity: 0.35;
  }
  .graph {
    border-radius: var(--radius-s);
    background: var(--bg-0);
    overflow: hidden;
  }
  svg {
    display: block;
    cursor: crosshair;
    touch-action: none;
  }
  .hist {
    fill: var(--bg-2);
  }
  .grid {
    stroke: var(--line);
    stroke-width: 1;
  }
  .diag {
    stroke: var(--line-strong);
    stroke-width: 1;
    stroke-dasharray: 2 3;
  }
  .line {
    fill: none;
    stroke-width: 1.6;
  }
  circle {
    fill: var(--bg-0);
    stroke-width: 1.5;
  }
</style>
