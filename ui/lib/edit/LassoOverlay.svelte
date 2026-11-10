<!--
  Select Object: draw a loose loop around something. On release the loop (in source uv) goes
  to `ondone`; Esc or a loop too small to mean anything cancels.
-->
<script lang="ts">
  import { toUv, type Vec2 } from "./masks";
  import type { Framing } from "./renderer";

  interface Props {
    frame: { x: number; y: number; w: number; h: number };
    framing: Framing;
    /** While detecting, the last loop stays on screen. */
    busy?: boolean;
    ondone: (lasso: [number, number][]) => void;
    oncancel: () => void;
  }
  let { frame, framing, busy = false, ondone, oncancel }: Props = $props();

  let el: SVGSVGElement;
  let points = $state<Vec2[]>([]);
  let drawing = false;

  function local(e: PointerEvent): Vec2 {
    const b = el.getBoundingClientRect();
    return [e.clientX - b.left, e.clientY - b.top];
  }

  function onDown(e: PointerEvent) {
    if (e.button !== 0 || busy) return;
    e.preventDefault();
    e.stopPropagation();
    el.setPointerCapture(e.pointerId);
    drawing = true;
    points = [local(e)];
  }

  function onMove(e: PointerEvent) {
    if (!drawing) return;
    const p = local(e);
    const q = points[points.length - 1];
    if (Math.hypot(p[0] - q[0], p[1] - q[1]) >= 4) points = [...points, p];
  }

  function onUp() {
    if (!drawing) return;
    drawing = false;
    const xs = points.map((p) => p[0]), ys = points.map((p) => p[1]);
    const big = Math.max(...xs) - Math.min(...xs) > 12 && Math.max(...ys) - Math.min(...ys) > 12;
    if (points.length < 6 || !big) {
      points = [];
      return;
    }
    const r4 = (v: number) => Math.round(v * 1e4) / 1e4;
    // Thin the loop (it only bounds the object) and map to source uv.
    const step = Math.max(1, Math.floor(points.length / 64));
    const lasso = points
      .filter((_, i) => i % step === 0)
      .map((p) => toUv(framing, [(p[0] - frame.x) / frame.w, (p[1] - frame.y) / frame.h]).map(r4) as [number, number]);
    ondone(lasso);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === "Escape") {
      e.stopPropagation();
      points = [];
      oncancel();
    }
  }

  $effect(() => {
    if (!busy) points = [];
  });

  const d = $derived(points.length ? `M${points.map((p) => p.join(",")).join("L")}${drawing ? "" : "Z"}` : "");
</script>

<svelte:window onkeydowncapture={onKey} />

<svg
  bind:this={el}
  class="lasso"
  class:busy
  onpointerdown={onDown}
  onpointermove={onMove}
  onpointerup={onUp}
  onpointercancel={onUp}
  role="application"
  aria-label="Circle an object"
>
  {#if d}
    <path class="dark" {d} />
    <path class="line" {d} />
  {:else if !busy}
    <foreignObject x={frame.x} y={frame.y + 12} width={frame.w} height="40">
      <div class="tip">Circle what you want to select · Esc to cancel</div>
    </foreignObject>
  {/if}
</svg>

<style>
  .lasso {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    touch-action: none;
    cursor: crosshair;
  }
  .lasso.busy {
    cursor: progress;
  }
  path {
    fill: none;
    stroke-linejoin: round;
    stroke-linecap: round;
  }
  .dark {
    stroke: rgba(0, 0, 0, 0.6);
    stroke-width: 3.5;
  }
  .line {
    stroke: #fff;
    stroke-width: 1.5;
    stroke-dasharray: 6 4;
  }
  .busy .line {
    animation: march 0.6s linear infinite;
  }
  @keyframes march {
    to {
      stroke-dashoffset: -10;
    }
  }
  .tip {
    width: max-content;
    margin: 0 auto;
    padding: 4px 12px;
    border-radius: 999px;
    background: rgba(20, 20, 20, 0.8);
    color: #eee;
    font-size: 12px;
  }
</style>
