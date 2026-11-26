<script lang="ts">
  // Handles for the selected mask, drawn over the image.
  // Linear: drag either end (or the middle pin to move it) · Radial: drag inside to move, the
  // edge handles to resize, the top knob to rotate · Brush: drag to paint (Alt: erase).
  import type { Mask } from "$lib/bindings/Mask";
  import type { Stroke } from "$lib/bindings/Stroke";
  import type { Framing } from "./renderer";
  import { isRaster, norm, toFrame, toUv, type BrushTool, type Vec2 } from "./masks";

  interface Props {
    mask: Mask;
    /** The frame on screen (CSS px, relative to the overlay). */
    frame: { x: number; y: number; w: number; h: number };
    framing: Framing;
    fullW: number;
    fullH: number;
    brush: BrushTool;
    /** Let pointer events through (e.g. Space held to pan). */
    passthrough?: boolean;
    onchange: (m: Mask) => void;
    /** The shape was nudged from the keyboard (after `onchange`): one of many, to be recorded
     *  as one step. */
    onnudge?: () => void;
    oncommit: () => void;
    /** A stroke grew from `a` to `b` (uv); `stroke` is the same object throughout. */
    onpaint: (stroke: Stroke, a: Vec2, b: Vec2) => void;
    onstroke: (stroke: Stroke) => void;
    /** A drag started (true) or ended (false), to show the mask while working. */
    oninteract?: (active: boolean) => void;
  }
  let {
    mask,
    frame,
    framing,
    fullW,
    fullH,
    brush,
    passthrough = false,
    onchange,
    onnudge,
    oncommit,
    onpaint,
    onstroke,
    oninteract,
  }: Props = $props();

  const nm = $derived(norm(fullW, fullH));
  /** Long-edge units per CSS pixel. */
  const unit = $derived(framing.w / frame.w / Math.max(fullW, fullH));

  const toScreen = (uv: Vec2): Vec2 => {
    const f = toFrame(framing, uv);
    return [frame.x + f[0] * frame.w, frame.y + f[1] * frame.h];
  };
  const toSource = (x: number, y: number): Vec2 =>
    toUv(framing, [(x - frame.x) / frame.w, (y - frame.y) / frame.h]);
  const qOf = (uv: Vec2): Vec2 => [uv[0] * nm[0], uv[1] * nm[1]];
  const uvOf = (q: Vec2): Vec2 => [q[0] / nm[0], q[1] / nm[1]];
  const r4 = (v: number) => Math.round(v * 1e4) / 1e4;
  const round = (p: Vec2): [number, number] => [r4(p[0]), r4(p[1])];

  let el: SVGSVGElement;
  let pointer = $state<Vec2 | null>(null);
  let altHeld = $state(false);

  function local(e: PointerEvent | MouseEvent): Vec2 {
    const b = el.getBoundingClientRect();
    return [e.clientX - b.left, e.clientY - b.top];
  }

  // ---- geometry on screen ----
  const linear = $derived.by(() => {
    if (mask.shape.kind !== "linear") return null;
    const a = toScreen(mask.shape.from as Vec2),
      b = toScreen(mask.shape.to as Vec2);
    const m: Vec2 = [(a[0] + b[0]) / 2, (a[1] + b[1]) / 2];
    const len = Math.hypot(b[0] - a[0], b[1] - a[1]) || 1;
    const p: Vec2 = [(-(b[1] - a[1]) / len) * 5000, ((b[0] - a[0]) / len) * 5000];
    const line = (c: Vec2) => ({
      x1: c[0] - p[0],
      y1: c[1] - p[1],
      x2: c[0] + p[0],
      y2: c[1] + p[1],
    });
    return { a, b, m, lines: [line(a), line(m), line(b)] };
  });

  const radial = $derived.by(() => {
    const s = mask.shape;
    if (s.kind !== "radial") return null;
    const c = qOf(s.center as Vec2);
    const t = (s.angle * Math.PI) / 180;
    const U: Vec2 = [Math.cos(t), Math.sin(t)],
      V: Vec2 = [-Math.sin(t), Math.cos(t)];
    const at = (k: number, sc = 1): Vec2 => {
      const [rx, ry] = s.radius;
      const q: Vec2 = [
        c[0] + sc * (rx * Math.cos(k) * U[0] + ry * Math.sin(k) * V[0]),
        c[1] + sc * (rx * Math.cos(k) * U[1] + ry * Math.sin(k) * V[1]),
      ];
      return toScreen(uvOf(q));
    };
    const path = (sc: number) => {
      let d = "";
      for (let i = 0; i <= 72; i++) {
        const p = at((i / 72) * 2 * Math.PI, sc);
        d += `${i ? "L" : "M"}${p[0].toFixed(1)},${p[1].toFixed(1)}`;
      }
      return d + "Z";
    };
    const knobQ: Vec2 = [
      c[0] - V[0] * (s.radius[1] + 22 * unit),
      c[1] - V[1] * (s.radius[1] + 22 * unit),
    ];
    return {
      c,
      U,
      V,
      centre: toScreen(s.center as Vec2),
      outer: path(1),
      inner: path(Math.max(0.02, 1 - s.feather)),
      handles: [
        { id: "rx", p: at(0) },
        { id: "rx", p: at(Math.PI) },
        { id: "ry", p: at(Math.PI / 2) },
        { id: "ry", p: at(-Math.PI / 2) },
      ],
      top: at(-Math.PI / 2),
      knob: toScreen(uvOf(knobQ)),
    };
  });

  // ---- dragging ----
  type Drag =
    | { kind: "from" | "to" | "move-linear"; start: Mask; at: Vec2; id: number }
    | { kind: "move-radial" | "rx" | "ry" | "rotate"; start: Mask; at: Vec2; id: number }
    | { kind: "paint"; stroke: Stroke; last: Vec2; id: number };
  let drag = $state.raw<Drag | null>(null);

  function near(p: Vec2, q: Vec2, r = 9) {
    return Math.hypot(p[0] - q[0], p[1] - q[1]) <= r;
  }

  function hit(x: number, y: number): Drag["kind"] | null {
    const p: Vec2 = [x, y];
    if (isRaster(mask)) return "paint";
    if (linear) {
      if (near(p, linear.a)) return "from";
      if (near(p, linear.b)) return "to";
      if (near(p, linear.m, 12)) return "move-linear";
      return null;
    }
    if (radial) {
      if (near(p, radial.knob)) return "rotate";
      for (const h of radial.handles) if (near(p, h.p)) return h.id as "rx" | "ry";
      // Inside the ellipse: move.
      const s = mask.shape as Extract<Mask["shape"], { kind: "radial" }>;
      const q = qOf(toSource(x, y));
      const d: Vec2 = [q[0] - radial.c[0], q[1] - radial.c[1]];
      const u = (d[0] * radial.U[0] + d[1] * radial.U[1]) / s.radius[0];
      const v = (d[0] * radial.V[0] + d[1] * radial.V[1]) / s.radius[1];
      if (u * u + v * v <= 1 || near(p, radial.centre, 12)) return "move-radial";
    }
    return null;
  }

  const cursor = $derived.by(() => {
    const k = drag?.kind ?? (isRaster(mask) ? "paint" : null);
    if (k === "paint") return "none";
    if (k === "rotate") return "alias";
    if (k === "move-linear" || k === "move-radial") return "move";
    if (k) return "grabbing";
    return undefined;
  });

  function onDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const [x, y] = local(e);
    const kind = hit(x, y);
    if (!kind) return;
    e.preventDefault();
    e.stopPropagation();
    el.setPointerCapture(e.pointerId);
    const uv = toSource(x, y);
    if (kind === "paint") {
      const stroke: Stroke = {
        points: [round(uv)],
        radius: r4(brush.size),
        feather: brush.feather,
        strength: brush.strength,
        erase: brush.erase || e.altKey,
      };
      drag = { kind, stroke, last: uv, id: e.pointerId };
      onpaint(stroke, uv, uv);
    } else {
      drag = { kind, start: $state.snapshot(mask) as Mask, at: uv, id: e.pointerId };
    }
    oninteract?.(true);
  }

  function onMove(e: PointerEvent) {
    const [x, y] = local(e);
    pointer = [x, y];
    altHeld = e.altKey;
    const d = drag;
    if (!d) return;
    const uv = toSource(x, y);
    if (d.kind === "paint") {
      // A new point every so often along the stroke: segments are swept, so this is smooth.
      const q0 = qOf(d.last),
        q1 = qOf(uv);
      if (Math.hypot(q1[0] - q0[0], q1[1] - q0[1]) < Math.max(d.stroke.radius * 0.15, unit * 2))
        return;
      d.stroke.points.push(round(uv));
      onpaint(d.stroke, d.last, uv);
      d.last = uv;
      return;
    }
    const start = d.start;
    const delta: Vec2 = [uv[0] - d.at[0], uv[1] - d.at[1]];
    if (start.shape.kind === "linear") {
      const s = start.shape;
      const shift = (p: Vec2) => round([p[0] + delta[0], p[1] + delta[1]]);
      const from = d.kind === "from" || d.kind === "move-linear" ? shift(s.from as Vec2) : s.from;
      const to = d.kind === "to" || d.kind === "move-linear" ? shift(s.to as Vec2) : s.to;
      onchange({ ...start, shape: { ...s, from, to } });
    } else if (start.shape.kind === "radial" && radial) {
      const s = start.shape;
      const c = qOf(s.center as Vec2);
      const q = qOf(uv);
      const dq: Vec2 = [q[0] - c[0], q[1] - c[1]];
      const t = (s.angle * Math.PI) / 180;
      const U: Vec2 = [Math.cos(t), Math.sin(t)],
        V: Vec2 = [-Math.sin(t), Math.cos(t)];
      const min = 4 * unit;
      if (d.kind === "move-radial") {
        onchange({
          ...start,
          shape: { ...s, center: round([s.center[0] + delta[0], s.center[1] + delta[1]]) },
        });
      } else if (d.kind === "rx") {
        const rx = Math.max(min, Math.abs(dq[0] * U[0] + dq[1] * U[1]));
        onchange({ ...start, shape: { ...s, radius: [r4(rx), s.radius[1]] } });
      } else if (d.kind === "ry") {
        const ry = Math.max(min, Math.abs(dq[0] * V[0] + dq[1] * V[1]));
        onchange({ ...start, shape: { ...s, radius: [s.radius[0], r4(ry)] } });
      } else if (d.kind === "rotate") {
        let a = (Math.atan2(dq[0], -dq[1]) * 180) / Math.PI;
        if (e.shiftKey) a = Math.round(a / 15) * 15;
        onchange({ ...start, shape: { ...s, angle: Math.round(a * 10) / 10 } });
      }
    }
  }

  function onUp() {
    const d = drag;
    if (!d) return;
    el.releasePointerCapture(d.id);
    drag = null;
    oninteract?.(false);
    if (d.kind === "paint") onstroke(d.stroke);
    else if (JSON.stringify(d.start) !== JSON.stringify($state.snapshot(mask))) oncommit();
  }

  // ---- keyboard ----
  // Each handle is a tab stop. Arrows move the ones that move (4 px on screen, 20 with Shift:
  // zoom in for finer); on a size or the angle, right and up are more, left and down less.
  type Handle = Exclude<Drag["kind"], "paint">;
  const DIRS: Record<string, Vec2> = {
    ArrowLeft: [-1, 0],
    ArrowRight: [1, 0],
    ArrowUp: [0, -1],
    ArrowDown: [0, 1],
  };
  function onHandleKey(e: KeyboardEvent, kind: Handle) {
    const dir = DIRS[e.key];
    if (!dir || drag || e.ctrlKey || e.metaKey || e.altKey) return;
    e.preventDefault();
    const start = $state.snapshot(mask) as Mask;
    const s = start.shape;
    const px = e.shiftKey ? 20 : 4;
    const more = dir[0] - dir[1];
    // The step in the photo's own coordinates: the view may be zoomed, turned or flipped.
    const o = toSource(frame.x, frame.y),
      p = toSource(frame.x + dir[0] * px, frame.y + dir[1] * px);
    const shift = (q: Vec2) => round([q[0] + p[0] - o[0], q[1] + p[1] - o[1]]);
    if (s.kind === "linear") {
      const from = kind === "to" ? s.from : shift(s.from as Vec2);
      const to = kind === "from" ? s.to : shift(s.to as Vec2);
      onchange({ ...start, shape: { ...s, from, to } });
    } else if (s.kind === "radial") {
      const size = (v: number) => r4(Math.max(4 * unit, v + more * px * unit));
      if (kind === "rx")
        onchange({ ...start, shape: { ...s, radius: [size(s.radius[0]), s.radius[1]] } });
      else if (kind === "ry")
        onchange({ ...start, shape: { ...s, radius: [s.radius[0], size(s.radius[1])] } });
      else if (kind === "rotate") {
        const a = s.angle + more * (e.shiftKey ? 15 : 1);
        const angle = Math.round((((((a + 180) % 360) + 360) % 360) - 180) * 10) / 10;
        onchange({ ...start, shape: { ...s, angle } });
      } else onchange({ ...start, shape: { ...s, center: shift(s.center as Vec2) } });
    } else return;
    onnudge?.();
  }
  /** Where a point on screen is in the photo as framed, in words. */
  const where = (p: Vec2) =>
    `${Math.round(((p[0] - frame.x) / frame.w) * 100)}% across, ${Math.round(((p[1] - frame.y) / frame.h) * 100)}% down`;
  const across = (p: Vec2) => Math.round(((p[0] - frame.x) / frame.w) * 100);
  /** A radius (long-edge units) as a percentage of the photo's long edge. */
  const ofEdge = (v: number) => Math.round(v * 100);

  const brushR = $derived(brush.size / unit);
  const erasing = $derived(brush.erase || altHeld || (drag?.kind === "paint" && drag.stroke.erase));
</script>

<svelte:window onkeydown={(e) => (altHeld = e.altKey)} onkeyup={(e) => (altHeld = e.altKey)} />

<svg
  bind:this={el}
  class="overlay"
  class:through={passthrough}
  class:paint={isRaster(mask)}
  class:dragging={drag !== null}
  style:cursor
  onpointerdown={onDown}
  onpointermove={onMove}
  onpointerup={onUp}
  onpointercancel={onUp}
  onpointerleave={() => (pointer = null)}
  role="application"
  aria-label="Mask"
>
  <defs>
    <clipPath id="mask-frame">
      <rect x={frame.x} y={frame.y} width={frame.w} height={frame.h} />
    </clipPath>
  </defs>
  {#if linear}
    <g clip-path="url(#mask-frame)">
      {#each linear.lines as l, i (i)}
        <line class="guide" class:mid={i === 1} class:faint={i === 2} {...l} />
        <line class="guide-dark" class:mid={i === 1} {...l} />
      {/each}
    </g>
    <circle
      class="handle"
      cx={linear.a[0]}
      cy={linear.a[1]}
      r="5"
      tabindex="0"
      role="slider"
      aria-label="Start of the gradient"
      aria-valuenow={across(linear.a)}
      aria-valuetext={where(linear.a)}
      onkeydown={(e) => onHandleKey(e, "from")}
    />
    <circle
      class="handle"
      cx={linear.b[0]}
      cy={linear.b[1]}
      r="5"
      tabindex="0"
      role="slider"
      aria-label="End of the gradient"
      aria-valuenow={across(linear.b)}
      aria-valuetext={where(linear.b)}
      onkeydown={(e) => onHandleKey(e, "to")}
    />
    <circle
      class="pin"
      cx={linear.m[0]}
      cy={linear.m[1]}
      r="7"
      tabindex="0"
      role="slider"
      aria-label="Position of the gradient"
      aria-valuenow={across(linear.m)}
      aria-valuetext={where(linear.m)}
      onkeydown={(e) => onHandleKey(e, "move-linear")}
    />
  {:else if radial}
    <path class="area" d={radial.outer} />
    <path class="guide-dark" d={radial.outer} />
    <path class="guide" d={radial.outer} />
    <path class="guide feather" d={radial.inner} />
    <line
      class="guide"
      x1={radial.top[0]}
      y1={radial.top[1]}
      x2={radial.knob[0]}
      y2={radial.knob[1]}
    />
    {@const s = mask.shape as Extract<Mask["shape"], { kind: "radial" }>}
    {#each radial.handles as h, i (i)}
      <!-- Two handles for each size: one of each pair is a tab stop. -->
      {#if i % 2 === 0}
        {@const v = s.radius[h.id === "rx" ? 0 : 1]}
        <circle
          class="handle"
          cx={h.p[0]}
          cy={h.p[1]}
          r="5"
          tabindex="0"
          role="slider"
          aria-label={h.id === "rx" ? "Width" : "Height"}
          aria-valuenow={ofEdge(v)}
          aria-valuetext="{ofEdge(v)}% of the photo’s long edge"
          onkeydown={(e) => onHandleKey(e, h.id as "rx" | "ry")}
        />
      {:else}
        <circle class="handle" cx={h.p[0]} cy={h.p[1]} r="5" />
      {/if}
    {/each}
    <circle
      class="knob"
      cx={radial.knob[0]}
      cy={radial.knob[1]}
      r="5"
      tabindex="0"
      role="slider"
      aria-label="Angle"
      aria-valuemin={-180}
      aria-valuemax={180}
      aria-valuenow={s.angle}
      aria-valuetext="{s.angle}°"
      onkeydown={(e) => onHandleKey(e, "rotate")}
    />
    <circle
      class="pin"
      cx={radial.centre[0]}
      cy={radial.centre[1]}
      r="7"
      tabindex="0"
      role="slider"
      aria-label="Position"
      aria-valuenow={across(radial.centre)}
      aria-valuetext={where(radial.centre)}
      onkeydown={(e) => onHandleKey(e, "move-radial")}
    />
  {:else if pointer && !passthrough}
    <circle class="brush-dark" cx={pointer[0]} cy={pointer[1]} r={brushR} />
    <circle class="brush" cx={pointer[0]} cy={pointer[1]} r={brushR} />
    {#if brush.feather > 0.02}
      <circle
        class="brush inner"
        cx={pointer[0]}
        cy={pointer[1]}
        r={brushR * (1 - brush.feather)}
      />
    {/if}
    <line class="brush" x1={pointer[0] - 4} y1={pointer[1]} x2={pointer[0] + 4} y2={pointer[1]} />
    {#if !erasing}<line
        class="brush"
        x1={pointer[0]}
        y1={pointer[1] - 4}
        x2={pointer[0]}
        y2={pointer[1] + 4}
      />{/if}
  {/if}
</svg>

<style>
  /* Only the handles take the pointer (elsewhere, clicks zoom and drags pan as usual); a brush
     paints anywhere. */
  .overlay {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    touch-action: none;
    pointer-events: none;
  }
  .overlay.paint,
  .overlay.dragging {
    pointer-events: auto;
  }
  .handle,
  .pin,
  .knob {
    pointer-events: all;
    cursor: grab;
  }
  .pin {
    cursor: move;
  }
  .knob {
    cursor: alias;
  }
  .area {
    fill: transparent;
    stroke: none;
    pointer-events: all;
    cursor: move;
  }
  .overlay.through,
  .overlay.through * {
    pointer-events: none;
  }
  .guide,
  .guide-dark,
  .brush,
  .brush-dark {
    fill: none;
    stroke-width: 1;
  }
  .guide,
  .brush {
    stroke: rgba(255, 255, 255, 0.9);
  }
  .guide-dark,
  .brush-dark {
    stroke: rgba(0, 0, 0, 0.45);
    stroke-width: 3;
  }
  .guide.mid {
    stroke-dasharray: 6 4;
  }
  .guide.faint {
    stroke: rgba(255, 255, 255, 0.45);
  }
  .guide-dark.mid {
    stroke-width: 0;
  }
  .guide.feather,
  .brush.inner {
    stroke-dasharray: 3 4;
    stroke: rgba(255, 255, 255, 0.6);
  }
  .handle,
  .knob {
    fill: #fff;
    stroke: rgba(0, 0, 0, 0.5);
  }
  .knob {
    fill: var(--focus, #6aa0ff);
  }
  .pin {
    fill: var(--focus, #6aa0ff);
    stroke: #fff;
    stroke-width: 2;
  }
  /* The handle that has the keyboard: a heavy ring (an outline isn't drawn around an SVG
     shape everywhere). */
  .handle:focus-visible,
  .knob:focus-visible,
  .pin:focus-visible {
    outline: none;
    stroke: var(--focus, #6aa0ff);
    stroke-width: 4;
    paint-order: stroke;
  }
  .knob:focus-visible,
  .pin:focus-visible {
    stroke: #fff;
  }
</style>
