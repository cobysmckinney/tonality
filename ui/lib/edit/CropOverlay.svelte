<script lang="ts">
  // Crop box over the straightened (uncropped) frame.
  // Drag corners/edges to resize · inside to move · outside to rotate · Ctrl+drag (or Level) to
  // draw a line that should be level.
  import type { Crop } from "$lib/bindings/Crop";
  import { MIN, clampInFrame, fits, levelAngle, shrinkToFit, towards, type Rect } from "./crop";

  interface Props {
    crop: Crop;
    /** Frame on screen (CSS px, relative to the overlay). */
    frame: { x: number; y: number; w: number; h: number };
    /** Turned frame size in pixels. */
    tw: number;
    th: number;
    /** Locked pixel aspect (w/h), or null for free. */
    aspect: number | null;
    level: boolean;
    onchange: (c: Crop) => void;
    onlevel?: () => void;
  }
  let { crop, frame, tw, th, aspect, level, onchange, onlevel }: Props = $props();

  const HANDLE = 12;

  const r = $derived(crop.rect as Rect);
  const box = $derived({
    x: frame.x + r[0] * frame.w,
    y: frame.y + r[1] * frame.h,
    w: r[2] * frame.w,
    h: r[3] * frame.h,
  });

  type Mode = "move" | "rotate" | "line" | `${"n" | "s" | ""}${"w" | "e" | ""}`;
  let drag = $state<{
    mode: Mode;
    x: number;
    y: number;
    start: Rect;
    angle: number;
    id: number;
  } | null>(null);
  let line = $state<{ x1: number; y1: number; x2: number; y2: number } | null>(null);
  let el: SVGSVGElement;

  function local(e: PointerEvent) {
    const b = el.getBoundingClientRect();
    return [e.clientX - b.left, e.clientY - b.top] as const;
  }

  function hitMode(x: number, y: number, e: PointerEvent): Mode {
    if (level || e.ctrlKey) return "line";
    const near = (a: number, b: number) => Math.abs(a - b) <= HANDLE;
    const inX = x >= box.x - HANDLE && x <= box.x + box.w + HANDLE;
    const inY = y >= box.y - HANDLE && y <= box.y + box.h + HANDLE;
    let v = "",
      hz = "";
    if (inX && near(y, box.y)) v = "n";
    else if (inX && near(y, box.y + box.h)) v = "s";
    if (inY && near(x, box.x)) hz = "w";
    else if (inY && near(x, box.x + box.w)) hz = "e";
    if (v || hz) return `${v}${hz}` as Mode;
    if (x > box.x && x < box.x + box.w && y > box.y && y < box.y + box.h) return "move";
    return "rotate";
  }

  let hover = $state<Mode>("move");
  const cursors: Record<string, string> = {
    move: "move",
    rotate: "alias",
    line: "crosshair",
    n: "ns-resize",
    s: "ns-resize",
    e: "ew-resize",
    w: "ew-resize",
    ne: "nesw-resize",
    sw: "nesw-resize",
    nw: "nwse-resize",
    se: "nwse-resize",
  };

  function onDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const [x, y] = local(e);
    const mode = hitMode(x, y, e);
    drag = { mode, x, y, start: [...r] as Rect, angle: crop.angle, id: e.pointerId };
    if (mode === "line") line = { x1: x, y1: y, x2: x, y2: y };
    el.setPointerCapture(e.pointerId);
    e.preventDefault();
  }

  function onMove(e: PointerEvent) {
    const [x, y] = local(e);
    if (!drag) {
      hover = hitMode(x, y, e);
      return;
    }
    const d = drag;
    if (d.mode === "line") {
      line = { ...line!, x2: x, y2: y };
      return;
    }
    if (d.mode === "rotate") {
      const cx = frame.x + frame.w / 2,
        cy = frame.y + frame.h / 2;
      const a0 = Math.atan2(d.y - cy, d.x - cx),
        a1 = Math.atan2(y - cy, x - cx);
      let delta = ((a1 - a0) * 180) / Math.PI;
      if (delta > 180) delta -= 360;
      if (delta < -180) delta += 360;
      const angle = Math.round(Math.max(-45, Math.min(45, d.angle + delta)) * 10) / 10;
      onchange({ ...crop, angle, rect: shrinkToFit(d.start, angle, tw, th) });
      return;
    }
    const dx = (x - d.x) / frame.w,
      dy = (y - d.y) / frame.h;
    const [sx, sy, sw, sh] = d.start;
    if (d.mode === "move") {
      // Slide along each axis separately, so the box glides along an edge it touches.
      const a = towards(d.start, clampInFrame([sx + dx, sy, sw, sh]), crop.angle, tw, th);
      const b = towards(a, clampInFrame([a[0], sy + dy, sw, sh]), crop.angle, tw, th);
      onchange({ ...crop, rect: b });
      return;
    }
    onchange({ ...crop, rect: resize(d.mode, d.start, dx, dy) });
  }

  /** Resize from a handle, keeping the opposite side (or corner) fixed. */
  function resize(mode: string, start: Rect, dx: number, dy: number): Rect {
    let [x0, y0] = [start[0], start[1]];
    let [x1, y1] = [start[0] + start[2], start[1] + start[3]];
    if (mode.includes("w")) x0 = Math.min(x1 - MIN, Math.max(0, x0 + dx));
    if (mode.includes("e")) x1 = Math.max(x0 + MIN, Math.min(1, x1 + dx));
    if (mode.includes("n")) y0 = Math.min(y1 - MIN, Math.max(0, y0 + dy));
    if (mode.includes("s")) y1 = Math.max(y0 + MIN, Math.min(1, y1 + dy));
    let next: Rect = [x0, y0, x1 - x0, y1 - y0];
    if (aspect) {
      // Frame-fraction height for a width under the locked pixel ratio.
      const hFor = (w: number) => (w * tw) / aspect / th;
      const wFor = (h: number) => (h * th * aspect) / tw;
      const horizontal = mode.includes("e") || mode.includes("w");
      const vertical = mode.includes("n") || mode.includes("s");
      let w = next[2],
        h = next[3];
      if (horizontal && (!vertical || Math.abs(dx) * tw >= Math.abs(dy) * th)) h = hFor(w);
      else w = wFor(h);
      // Anchor: the fixed corner, or the middle of the fixed side.
      const ax = mode.includes("w")
        ? start[0] + start[2]
        : mode.includes("e")
          ? start[0]
          : start[0] + start[2] / 2;
      const ay = mode.includes("n")
        ? start[1] + start[3]
        : mode.includes("s")
          ? start[1]
          : start[1] + start[3] / 2;
      const nx = mode.includes("w") ? ax - w : mode.includes("e") ? ax : ax - w / 2;
      const ny = mode.includes("n") ? ay - h : mode.includes("s") ? ay : ay - h / 2;
      next = [nx, ny, w, h];
      if (next[0] < 0 || next[1] < 0 || next[0] + w > 1 || next[1] + h > 1) {
        // Too big for the frame: stay where we were rather than distort.
        return towards(start, next, crop.angle, tw, th);
      }
    }
    return fits(next, crop.angle, tw, th) ? next : towards(start, next, crop.angle, tw, th);
  }

  function onUp() {
    if (!drag) return;
    el.releasePointerCapture(drag.id);
    if (drag.mode === "line" && line) {
      const corr = levelAngle(line.x1, line.y1, line.x2, line.y2);
      if (corr !== null) {
        const angle = Math.round(Math.max(-45, Math.min(45, crop.angle + corr)) * 100) / 100;
        onchange({ ...crop, angle, rect: shrinkToFit(crop.rect as Rect, angle, tw, th) });
      }
      line = null;
      onlevel?.();
    }
    drag = null;
  }

  const thirds = [1 / 3, 2 / 3];
  const showGrid = $derived(drag !== null && drag.mode !== "line");
</script>

<svg
  bind:this={el}
  class="overlay"
  style:cursor={cursors[drag?.mode ?? hover] ?? "default"}
  onpointerdown={onDown}
  onpointermove={onMove}
  onpointerup={onUp}
  onpointercancel={onUp}
  role="application"
  aria-label="Crop"
>
  <path
    class="shade"
    fill-rule="evenodd"
    d="M0,0H10000V10000H0Z M{box.x},{box.y}h{box.w}v{box.h}h{-box.w}Z"
  />
  <rect class="border" x={box.x} y={box.y} width={box.w} height={box.h} />
  {#each thirds as t (t)}
    <line
      class="third"
      class:on={showGrid}
      x1={box.x + box.w * t}
      y1={box.y}
      x2={box.x + box.w * t}
      y2={box.y + box.h}
    />
    <line
      class="third"
      class:on={showGrid}
      x1={box.x}
      y1={box.y + box.h * t}
      x2={box.x + box.w}
      y2={box.y + box.h * t}
    />
  {/each}
  {#each [[0, 0, 1, 1], [1, 0, -1, 1], [0, 1, 1, -1], [1, 1, -1, -1]] as [cx, cy, sx, sy] (`${cx}${cy}`)}
    {@const px = box.x + cx * box.w}
    {@const py = box.y + cy * box.h}
    <path class="corner" d="M{px},{py + sy * 16}V{py}H{px + sx * 16}" />
  {/each}
  {#each [[0.5, 0, 1, 0], [0.5, 1, 1, 0], [0, 0.5, 0, 1], [1, 0.5, 0, 1]] as [cx, cy, hx, hy] (`${cx}${cy}`)}
    {@const px = box.x + cx * box.w}
    {@const py = box.y + cy * box.h}
    <line class="edge" x1={px - hx * 10} y1={py - hy * 10} x2={px + hx * 10} y2={py + hy * 10} />
  {/each}
  {#if line}
    <line class="level" x1={line.x1} y1={line.y1} x2={line.x2} y2={line.y2} />
  {/if}
</svg>

<style>
  .overlay {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    touch-action: none;
  }
  .shade {
    fill: rgba(10, 10, 10, 0.62);
  }
  .border {
    fill: none;
    stroke: rgba(255, 255, 255, 0.85);
    stroke-width: 1;
  }
  .third {
    stroke: rgba(255, 255, 255, 0);
    stroke-width: 1;
    transition: stroke 120ms;
  }
  .third.on {
    stroke: rgba(255, 255, 255, 0.35);
  }
  .corner,
  .edge {
    fill: none;
    stroke: #fff;
    stroke-width: 3;
  }
  .level {
    stroke: var(--focus, #6aa0ff);
    stroke-width: 1.5;
    stroke-dasharray: 5 4;
  }
</style>
