<!--
  Navigator: while zoomed in, a small overview of the whole frame in the canvas's corner, with a
  rectangle for the part on screen. Drag the rectangle, or press anywhere on the overview, to pan.
-->
<script lang="ts">
  import { untrack } from "svelte";
  import type { PreviewRenderer } from "./renderer";
  import { centerAt, grabOffset, overviewSize, type Box } from "./navigator";

  interface Props {
    renderer: PreviewRenderer;
    /** The frame's rect on the canvas: changes whenever the view pans or zooms. */
    frame: Box;
    /** Bumped whenever the picture changes (new params or photo). */
    epoch: number;
  }
  let { renderer, frame, epoch }: Props = $props();

  /** Long edge of the overview, CSS px. */
  const EDGE = 160;

  let el = $state<HTMLCanvasElement>();
  let size = $state({ w: EDGE, h: EDGE });
  const vis = $derived.by(() => {
    void frame;
    return renderer.visible();
  });

  // Redraw the overview shortly after the picture settles (it's one small offscreen render).
  $effect(() => {
    void epoch;
    const r = renderer;
    const t = setTimeout(
      () =>
        untrack(() => {
          const dpr = window.devicePixelRatio || 1;
          const img = r.renderOverview(Math.round(EDGE * dpr));
          if (!img || !el) return;
          el.width = img.width;
          el.height = img.height;
          el.getContext("2d")?.putImageData(img, 0, 0);
          size = overviewSize(img.width, img.height, EDGE);
        }),
      150,
    );
    return () => clearTimeout(t);
  });

  let grab = $state<{ x: number; y: number; id: number } | null>(null);

  function at(e: PointerEvent) {
    const b = el!.getBoundingClientRect();
    return [e.clientX - b.left, e.clientY - b.top] as const;
  }

  function onDown(e: PointerEvent) {
    if (e.button !== 0 || !el) return;
    e.stopPropagation();
    const [x, y] = at(e);
    const off = grabOffset(x, y, size, vis);
    grab = { ...off, id: e.pointerId };
    el.setPointerCapture(e.pointerId);
    const c = centerAt(x, y, size, vis, off);
    renderer.panTo(c.x, c.y);
  }

  function onMove(e: PointerEvent) {
    if (!grab) return;
    const [x, y] = at(e);
    const c = centerAt(x, y, size, vis, grab);
    renderer.panTo(c.x, c.y);
  }

  function onUp() {
    if (!grab || !el) return;
    el.releasePointerCapture(grab.id);
    grab = null;
  }
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="nav"
  style:width="{size.w}px"
  style:height="{size.h}px"
  onwheel={(e) => (e.preventDefault(), e.stopPropagation())}
>
  <canvas
    bind:this={el}
    class:grabbing={grab}
    onpointerdown={onDown}
    onpointermove={onMove}
    onpointerup={onUp}
    onpointercancel={onUp}
  ></canvas>
  <div
    class="view"
    style:left="{vis.x * 100}%"
    style:top="{vis.y * 100}%"
    style:width="{vis.w * 100}%"
    style:height="{vis.h * 100}%"
  ></div>
</div>

<style>
  .nav {
    position: absolute;
    right: var(--space-3);
    bottom: var(--space-3);
    border-radius: var(--radius-s);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    overflow: hidden;
  }
  canvas {
    display: block;
    width: 100%;
    height: 100%;
    cursor: pointer;
    touch-action: none;
  }
  canvas.grabbing {
    cursor: grabbing;
  }
  .view {
    position: absolute;
    box-sizing: border-box;
    border: 1.5px solid var(--text-0);
    box-shadow: 0 0 0 9999px rgba(0, 0, 0, 0.35);
    pointer-events: none;
  }
</style>
