<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "$lib/api";
  import type { EditState } from "$lib/bindings/EditState";
  import type { ImageInfo } from "$lib/bindings/ImageInfo";
  import { MAX_ZOOM, MIN_ZOOM, PreviewRenderer, type Histogram } from "./renderer";
  import { attachRenderer } from "./versions";
  import type { Crop } from "$lib/bindings/Crop";
  import CropOverlay from "./CropOverlay.svelte";
  import { FULL, turnedSize } from "./crop";
  import MaskOverlay from "./MaskOverlay.svelte";
  import { brushKey, brushLayer, maskSize, masksOf, shaderIndex, type BrushLayers } from "./masks";
  import { session } from "./session.svelte";
  import type { Mask } from "$lib/bindings/Mask";
  import type { Stroke } from "$lib/bindings/Stroke";

  interface Props {
    image: ImageInfo;
    pixels: Uint16Array;
    edit: EditState;
    /** Show the unedited image (before/after). */
    before?: boolean;
    beforeEdit: EditState;
    /** Another version to show side by side, labelled `compare.label`. */
    compare?: { label: string; current: string; state: EditState } | null;
    oncloseCompare?: () => void;
    /** Crop mode: show the whole straightened frame with the crop box over it. */
    cropping?: boolean;
    cropAspect?: number | null;
    level?: boolean;
    oncrop?: (c: Crop) => void;
    onlevel?: () => void;
    onhistogram?: (h: Histogram) => void;
  }
  let {
    image,
    pixels,
    edit,
    before = false,
    beforeEdit,
    compare = null,
    oncloseCompare,
    cropping = false,
    cropAspect = null,
    level = false,
    oncrop,
    onlevel,
    onhistogram,
  }: Props = $props();
  let frameRect = $state({ x: 0, y: 0, w: 0, h: 0 });
  const turned = $derived(turnedSize(edit.crop, image.fullWidth, image.fullHeight));

  let canvas: HTMLCanvasElement;
  let renderer = $state<PreviewRenderer | null>(null);
  let error = $state<string | null>(null);
  let scale = $state(1);
  let fit = $state(true);
  let loadingDetail = $state(false);
  let devZoom: number | null = null;

  onMount(() => {
    let disposed = false;
    const ro = new ResizeObserver(() => {
      if (!renderer) return;
      renderer.setZoom(renderer.isFit ? null : renderer.scale); // re-clamp and refresh the readout
    });
    ro.observe(canvas);
    api
      .previewShaders()
      .then(async (src) => {
        if (disposed) return;
        const r = new PreviewRenderer(canvas, src);
        devZoom = await api.devZoom();
        if (await api.devSnapshotPath()) {
          r.onDrawn = (c) => {
            if (devZoom && !r.hasDetail) return; // wait for the full-res pixels
            r.onDrawn = null;
            api.devSaveSnapshot(c.toDataURL("image/png"));
          };
        }
        r.onHistogram = (h) => onhistogram?.(h);
        r.onLayout = (rect) => (frameRect = rect);
        r.onView = (s, f) => {
          scale = s;
          fit = f;
        };
        r.onNeedDetail = async (req) => {
          const id = image.id;
          loadingDetail = true;
          try {
            const d = await api.detailRegion(id, req);
            if (renderer === r && image.id === id) r.setDetail(d.region, req.level, d.width, d.height, d.pixels);
          } catch (e) {
            console.warn("[detail]", e);
          } finally {
            loadingDetail = false;
          }
        };
        renderer = r;
      })
      .catch((e) => (error = String(e)));
    return () => {
      disposed = true;
      ro.disconnect();
      renderer?.dispose();
    };
  });

  // Upload pixels whenever a new image opens.
  $effect(() => {
    const r = renderer;
    if (!r) return;
    r.setImage(image.width, image.height, pixels, image.fullWidth, image.fullHeight);
    r.setZoom(devZoom);
  });

  // ---- zoom & pan ----
  // Click toggles fit ↔ 100% at the cursor, drag pans, the wheel zooms around the cursor.
  let drag: { x: number; y: number; moved: boolean; id: number } | null = null;
  let panning = $state(false);

  function local(e: MouseEvent) {
    const b = canvas.getBoundingClientRect();
    return [e.clientX - b.left, e.clientY - b.top] as const;
  }

  /** Toggle between fit and 1:1 (one image pixel per screen pixel). */
  export function toggleZoom(at?: readonly [number, number]) {
    const r = renderer;
    if (!r) return;
    r.setZoom(r.isFit ? 1 : null, at?.[0], at?.[1]);
  }

  export function zoomBy(factor: number, at?: readonly [number, number]) {
    const r = renderer;
    if (!r) return;
    r.setZoom(Math.min(MAX_ZOOM, Math.max(MIN_ZOOM, r.scale * factor)), at?.[0], at?.[1]);
  }

  export function setZoom(z: number | null) {
    renderer?.setZoom(z);
  }

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0 || !renderer || cropping) return;
    const [x, y] = local(e);
    if (renderer.isFit && !renderer.hit(x, y)) return;
    drag = { x: e.clientX, y: e.clientY, moved: false, id: e.pointerId };
    canvas.setPointerCapture(e.pointerId);
  }

  function onPointerMove(e: PointerEvent) {
    if (!drag || !renderer) return;
    const dx = e.clientX - drag.x;
    const dy = e.clientY - drag.y;
    if (!drag.moved && Math.hypot(dx, dy) < 4) return;
    drag.moved = true;
    panning = true;
    renderer.pan(dx, dy);
    drag.x = e.clientX;
    drag.y = e.clientY;
  }

  function onPointerUp(e: PointerEvent) {
    if (!drag) return;
    canvas.releasePointerCapture(drag.id);
    if (!drag.moved && e.type === "pointerup") toggleZoom(local(e));
    drag = null;
    panning = false;
  }

  function onWheel(e: WheelEvent) {
    e.preventDefault();
    if (cropping) return;
    if (e.altKey && session.masking) {
      // Alt+wheel sizes the brush.
      const b = session.brush;
      b.size = Math.min(0.3, Math.max(0.003, b.size * Math.exp(-e.deltaY * 0.002)));
      return;
    }
    const lines = e.deltaMode === 1 ? e.deltaY * 16 : e.deltaY;
    zoomBy(Math.exp(-lines * 0.0025), local(e));
  }

  const zoomLabel = $derived(fit ? "Fit" : `${Math.round(scale * 100)}%`);

  // Latest-wins parameter pipelines (the main view, and the compare pane): at most one request
  // in flight each, always ending on the newest state. The curve LUT is only refetched when the
  // curve itself changes.
  function pipeline(
    apply: (r: PreviewRenderer, params: Float32Array, curve: Uint16Array | null, brush: BrushLayers | null | undefined) => void,
  ) {
    let inflight = false;
    let pending: EditState | null = null;
    let curveKey = "";
    let brushK: string | null = null;
    async function pump(r: PreviewRenderer, id: number) {
      if (inflight) return;
      inflight = true;
      try {
        while (pending) {
          const next = pending;
          pending = null;
          const key = JSON.stringify(next.curve);
          // Brush rasters only when strokes change (undefined: unchanged; null: none).
          const bk = `${id}:${brushKey(next)}`;
          const [params, curve, brush] = await Promise.all([
            api.developParams(id, next),
            key !== curveKey ? api.developCurve(next) : null,
            bk === brushK ? undefined : brushKey(next) ? api.maskLayers(id, next) : null,
          ]);
          if (curve) curveKey = key;
          brushK = bk;
          apply(r, params, curve, brush);
        }
      } catch (e) {
        error = String(e);
      } finally {
        inflight = false;
      }
    }
    return (r: PreviewRenderer, id: number, state: EditState) => {
      pending = state;
      pump(r, id);
    };
  }
  const main = pipeline((r, params, curve, brush) => {
    if (curve) r.setCurve(curve);
    if (brush !== undefined) r.setBrush("main", brush);
    r.setParams(params);
    session.framing = r.framing;
  });
  const side = pipeline((r, params, curve, brush) => {
    if (!comparing) return;
    if (brush !== undefined) r.setBrush("compare", brush);
    r.setCompare(params, curve);
  });

  // ---- masks ----
  const masks = $derived(masksOf(edit));
  const selected = $derived(session.masking && !before ? session.selected : null);
  let interacting = $state(false);
  let spaceHeld = $state(false);

  $effect(() => {
    const r = renderer;
    if (!r) return;
    const show = selected !== null && (session.maskOverlay || interacting);
    r.setOverlay(show && selected !== null ? shaderIndex(masks, selected) : null);
  });

  function setMask(i: number, m: Mask) {
    if (!edit.masks) return;
    edit.masks[i] = m;
  }

  function paintLive(i: number, stroke: Stroke, a: readonly number[], b: readonly number[]) {
    const r = renderer;
    if (!r) return;
    r.paintLive(brushLayer(masks, i), stroke, a, b, maskSize(image.fullWidth, image.fullHeight));
  }

  function endStroke(i: number, stroke: Stroke) {
    renderer?.endLive();
    const m = edit.masks?.[i];
    if (!m || m.shape.kind !== "brush") return;
    m.shape.strokes.push(stroke);
    session.commit(null);
  }

  function onKey(e: KeyboardEvent) {
    if (e.key === " " && session.masking && !(e.target instanceof HTMLInputElement)) {
      spaceHeld = e.type === "keydown";
      if (spaceHeld) e.preventDefault();
    }
  }

  $effect(() => {
    const state = $state.snapshot(before ? beforeEdit : edit) as EditState;
    // While cropping, show the whole (straightened) frame; the overlay shows the crop.
    if (cropping && !before) state.crop = { ...state.crop, rect: [...FULL] };
    const r = renderer;
    if (r) main(r, image.id, state);
  });

  $effect(() => {
    if (cropping) renderer?.setZoom(null);
  });

  let comparing = false;
  $effect(() => {
    const r = renderer;
    const state = compare ? ($state.snapshot(compare.state) as EditState) : null;
    if (!r) return;
    comparing = state !== null;
    if (state) side(r, image.id, state);
    else r.setCompare(null);
  });

  // Branch / library thumbnails are rendered with this renderer.
  $effect(() => {
    const r = renderer;
    const id = image.id;
    if (!r) return;
    attachRenderer(r, id);
    return () => attachRenderer(null, null);
  });
</script>

<svelte:window onkeydown={onKey} onkeyup={onKey} onblur={() => (spaceHeld = false)} />

<div class="wrap" onwheel={onWheel}>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <canvas
    bind:this={canvas}
    class:zoomed={!fit}
    class:panning
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
  ></canvas>
  {#if cropping && oncrop && frameRect.w > 0}
    <CropOverlay crop={edit.crop} frame={frameRect} tw={turned[0]} th={turned[1]} aspect={cropAspect} {level} onchange={oncrop} {onlevel} />
  {/if}
  {#if selected !== null && masks[selected] && renderer && frameRect.w > 0}
    {@const i = selected}
    <MaskOverlay
      mask={masks[i]}
      frame={frameRect}
      framing={session.framing ?? renderer.framing}
      fullW={image.fullWidth}
      fullH={image.fullHeight}
      brush={session.brush}
      passthrough={spaceHeld}
      onchange={(m) => setMask(i, m)}
      oncommit={() => session.commit(null)}
      onpaint={(stroke, a, b) => paintLive(i, stroke, a, b)}
      onstroke={(stroke) => endStroke(i, stroke)}
      oninteract={(v) => (interacting = v)}
    />
  {/if}
  <div class="zoom" class:hidden={cropping}>
    <button onclick={() => toggleZoom()} title="Toggle fit / 100% (Z)">{zoomLabel}</button>
    {#if loadingDetail}<span class="loading">Loading detail…</span>{/if}
  </div>
  {#if compare}
    <div class="pane-label left">{before ? "Before" : compare.current}</div>
    <div class="pane-label right">
      {compare.label}
      {#if oncloseCompare}<button onclick={oncloseCompare} title="Stop comparing (Esc)">×</button>{/if}
    </div>
  {:else if before}<div class="badge">Before</div>{/if}
  {#if error}<div class="error">{error}</div>{/if}
</div>

<style>
  .wrap {
    position: absolute;
    inset: 0;
  }
  canvas {
    width: 100%;
    height: 100%;
    display: block;
    cursor: zoom-in;
    touch-action: none;
  }
  canvas.zoomed {
    cursor: grab;
  }
  canvas.panning {
    cursor: grabbing;
  }
  .pane-label {
    position: absolute;
    top: var(--space-2);
    display: flex;
    gap: 6px;
    align-items: center;
    padding: 2px 10px;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.55);
    font-size: var(--fs-xs);
    color: var(--text-0);
    max-width: 40%;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pane-label.left {
    left: var(--space-3);
  }
  .pane-label.right {
    left: calc(50% + var(--space-3));
  }
  .pane-label button {
    color: var(--text-2);
  }
  .pane-label button:hover {
    color: var(--text-0);
  }
  .zoom {
    position: absolute;
    top: var(--space-2);
    right: var(--space-3);
    display: flex;
    gap: var(--space-2);
    align-items: center;
    font-size: var(--fs-xs);
  }
  .zoom.hidden {
    display: none;
  }
  .zoom button {
    min-width: 44px;
    padding: 2px 8px;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.45);
    color: var(--text-1);
    font-family: var(--font-num);
  }
  .zoom button:hover {
    color: var(--text-0);
  }
  .loading {
    color: var(--text-2);
  }
  .badge {
    position: absolute;
    top: var(--space-3);
    left: 50%;
    transform: translateX(-50%);
    padding: 2px 10px;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.6);
    font-size: var(--fs-xs);
    color: var(--text-1);
    letter-spacing: 0.04em;
  }
  .error {
    position: absolute;
    bottom: var(--space-3);
    left: var(--space-3);
    right: var(--space-3);
    color: var(--danger);
    font-family: var(--font-num);
    font-size: var(--fs-xs);
    white-space: pre-wrap;
  }
</style>
