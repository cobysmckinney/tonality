<script lang="ts">
  // Point tone curve: master RGB plus per-channel curves.
  // Click the curve to add a point · drag points · double-click (or drag off the graph) to remove.
  // Each point is also a tab stop: arrows move it, Enter adds one after it, Delete removes it.
  import { tick } from "svelte";
  import type { ToneCurve } from "$lib/bindings/ToneCurve";
  import { curve as evaluator, dragPoint, IDENTITY, isIdentity, type Point } from "./curve";
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
  // `start` is the points at pointer-down; every move is computed from it.
  let drag: { index: number; start: Point[] } | null = null;

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
    drag = { index, start: pts };
    svg.setPointerCapture(e.pointerId);
  }

  function onMove(e: PointerEvent) {
    if (!drag) return;
    emit(dragPoint(drag.start, drag.index, toUnit(e)));
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

  // ---- keyboard ----
  let dots = $state<SVGCircleElement[]>([]);
  const uid = $props.id();
  const pct = (v: number) => `${Math.round(v * 100)}%`;
  const STEPS: Record<string, Point> = {
    ArrowLeft: [-1, 0],
    ArrowRight: [1, 0],
    ArrowUp: [0, 1],
    ArrowDown: [0, -1],
  };

  async function onPointKey(e: KeyboardEvent, i: number) {
    if (e.ctrlKey || e.metaKey || e.altKey) return;
    const by = STEPS[e.key];
    const remove = e.key === "Delete" || e.key === "Backspace";
    if (!by && !remove && e.key !== "Enter") return;
    // Ours, whatever comes of it: Delete on an end point mustn't go on to delete a mask.
    e.preventDefault();
    e.stopPropagation();
    const last = points.length - 1;
    let focus = i;
    if (by) {
      const step = e.shiftKey ? 0.05 : 0.01;
      emit(dragPoint(points, i, [points[i][0] + by[0] * step, points[i][1] + by[1] * step]));
    } else if (remove) {
      if (i === 0 || i === last) return; // the ends stay
      emit(points.filter((_, j) => j !== i));
    } else {
      // A new point halfway to the next one (from the last, halfway back to the one before).
      const [a, b] = i < last ? [i, i + 1] : [i - 1, i];
      if (points[b][0] - points[a][0] < 0.04) return; // no room between them
      const x = (points[a][0] + points[b][0]) / 2;
      const pts = points.map((q) => [...q] as Point);
      pts.splice(b, 0, [x, f(x)]);
      emit(pts);
      focus = b;
    }
    oncommit();
    await tick();
    dots[focus]?.focus();
  }
</script>

<div class="curve">
  <div class="tabs">
    {#each channels as c (c.id)}
      <button
        class:active={channel === c.id}
        aria-pressed={channel === c.id}
        onclick={() => (channel = c.id)}
        style:--c={c.colour}
      >
        {c.label}{#if edited(c.id)}<span class="dot"></span><span class="sr-only">
            , edited
          </span>{/if}
      </button>
    {/each}
    <button
      class="reset"
      onclick={resetChannel}
      disabled={!edited(channel)}
      title="Reset this curve"
    >
      Reset
    </button>
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
        <circle
          bind:this={dots[i]}
          cx={x * size}
          cy={(1 - y) * size}
          r="4"
          style:stroke={colour}
          tabindex="0"
          role="slider"
          aria-label="Point {i + 1} of {points.length}"
          aria-valuemin={0}
          aria-valuemax={100}
          aria-valuenow={Math.round(y * 100)}
          aria-valuetext="input {pct(x)}, output {pct(y)}"
          aria-describedby="{uid}-keys"
          onkeydown={(e) => onPointKey(e, i)}
        />
      {/each}
    </svg>
    <p class="sr-only" id="{uid}-keys">
      Arrow keys move the point, further with Shift. Enter adds a point after it. Delete removes it.
    </p>
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
  /* Filled in while it has the keyboard (an outline isn't drawn around an SVG shape
     everywhere). */
  circle:focus-visible {
    outline: none;
    fill: var(--text-0);
    stroke-width: 3;
  }
</style>
