<script lang="ts">
  import { onMount, untrack } from "svelte";
  import { api } from "$lib/api";
  import { toasts } from "$lib/toasts/toasts.svelte";
  import type { EditState } from "$lib/bindings/EditState";
  import type { ImageInfo } from "$lib/bindings/ImageInfo";
  import { MAX_ZOOM, MIN_ZOOM, PreviewRenderer, type Histogram } from "./renderer";
  import { attachRenderer } from "./versions";
  import type { Crop } from "$lib/bindings/Crop";
  import CropOverlay from "./CropOverlay.svelte";
  import { FULL, turnedSize } from "./crop";
  import MaskOverlay from "./MaskOverlay.svelte";
  import LassoOverlay from "./LassoOverlay.svelte";
  import Navigator from "./Navigator.svelte";
  import {
    MASK_THUMB_EDGE,
    brushKey,
    brushLayer,
    maskSize,
    masksOf,
    shaderIndex,
    usesPresenceMaps,
    withoutStrokes,
    type BrushLayers,
  } from "./masks";
  import { session } from "./session.svelte";
  import { focusedControlOwnsKey } from "$lib/keys";
  import type { Mask } from "$lib/bindings/Mask";
  import type { Stroke } from "$lib/bindings/Stroke";

  interface Props {
    image: ImageInfo;
    pixels: Uint16Array;
    edit: EditState;
    /** Show the unedited image (before/after). */
    before?: boolean;
    beforeEdit: EditState;
    /** Another version to show side by side, labelled `compare.label`. With `image` (and its
     * preview `pixels`), another photo instead: on the left, with this one on the right. */
    compare?: {
      label: string;
      current: string;
      state: EditState;
      image?: ImageInfo;
      pixels?: Uint16Array;
    } | null;
    oncloseCompare?: () => void;
    /** Crop mode: show the whole straightened frame with the crop box over it. */
    cropping?: boolean;
    cropAspect?: number | null;
    level?: boolean;
    oncrop?: (c: Crop) => void;
    onlevel?: () => void;
    onhistogram?: (h: Histogram) => void;
    /**
     * Pick mode (the white balance eyedropper): a crosshair, and a click calls this with the
     * frame position under it (0..1 across the photo as framed) instead of zooming.
     */
    onpick?: ((at: [number, number]) => void) | null;
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
    onpick = null,
  }: Props = $props();
  let frameRect = $state({ x: 0, y: 0, w: 0, h: 0 });
  const turned = $derived(turnedSize(edit.crop, image.fullWidth, image.fullHeight));

  let canvas: HTMLCanvasElement;
  let renderer = $state<PreviewRenderer | null>(null);
  /** The toast for a preview that failed to render, taken down once one renders again. */
  let renderError: number | null = null;
  let scale = $state(1);
  let fit = $state(true);
  let loadingDetail = $state(false);
  let devZoom: number | null = null;

  onMount(() => {
    let disposed = false;
    let detailSeq = 0;
    const ro = new ResizeObserver(() => {
      if (!renderer) return;
      renderer.setZoom(renderer.isFit ? null : renderer.scale); // re-clamp and refresh the readout
    });
    ro.observe(canvas);

    async function init() {
      const src = await api.previewShaders();
      if (disposed) return;
      const r = new PreviewRenderer(canvas, src);
      devZoom = await api.devZoom();
      const snapshot = await api.devSnapshotPath();
      // Unmounted while we waited: nobody else will dispose it.
      if (disposed) return r.dispose();
      if (snapshot) {
        r.onDrawn = (c) => {
          if (devZoom && !r.hasDetail) return; // wait for the full-res pixels
          r.onDrawn = null;
          api.devSaveSnapshot(c.toDataURL("image/png"));
        };
      }
      r.onHistogram = (h) => onhistogram?.(h);
      r.onLayout = (rect) => {
        frameRect = rect;
        probe(); // the photo moved under the pointer
      };
      r.onView = (s, f) => {
        scale = s;
        fit = f;
      };
      r.onNeedDetail = async (req) => {
        const id = image.id;
        // Latest wins: an older region coming back late must not replace a newer one.
        const seq = ++detailSeq;
        loadingDetail = true;
        try {
          const d = await api.detailRegion(id, req);
          if (seq === detailSeq && renderer === r && image.id === id)
            r.setDetail(d.region, req.level, d.width, d.height, d.pixels);
        } catch (e) {
          // Without this the 100% view stays the preview, scaled up, and nothing says so.
          if (seq === detailSeq && image.id === id)
            toasts.error("Couldn’t load full-resolution detail", e);
        } finally {
          if (seq === detailSeq) loadingDetail = false;
        }
      };
      renderer = r;
    }
    init().catch((e) => toasts.error("Couldn’t start the preview", e));

    // The GPU can drop the context (driver reset, suspend/resume): build a new renderer once
    // it's back; the effects below re-upload the photo and its params.
    const lost = (e: Event) => {
      e.preventDefault(); // allows the restore
      renderer?.dispose();
      renderer = null;
    };
    const restored = () => {
      if (!disposed) init().catch((e) => toasts.error("Couldn’t start the preview", e));
    };
    canvas.addEventListener("webglcontextlost", lost);
    canvas.addEventListener("webglcontextrestored", restored);
    return () => {
      disposed = true;
      ro.disconnect();
      canvas.removeEventListener("webglcontextlost", lost);
      canvas.removeEventListener("webglcontextrestored", restored);
      // Let the library thumbnail render first (cleanup order between effects isn't ours).
      attachRenderer(null, null);
      renderer?.dispose({ releaseContext: true });
    };
  });

  // Upload pixels whenever a new image opens.
  $effect(() => {
    const r = renderer;
    if (!r) return;
    // Leaving the previous photo: let its library thumbnail render while its pixels are still
    // uploaded (the attach effect below only lets go after this upload).
    untrack(() => attachRenderer(null, null));
    r.setImage(image.width, image.height, pixels, image.fullWidth, image.fullHeight, image.id);
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
    if (onpick) {
      // `frameRect` is the photo within its pane: the left one when comparing versions, the
      // right one beside another photo.
      const r = frameRect;
      const f: [number, number] = [(x - renderer.mainPaneX - r.x) / r.w, (y - r.y) / r.h];
      // Zoomed in, the photo's frame is wider than its pane: a click over the other pane is
      // still "inside the frame", at a spot that isn't under the pointer (or on screen).
      if (!renderer.inMainPane(x)) return;
      if (r.w > 0 && r.h > 0 && f.every((v) => v >= 0 && v <= 1)) onpick(f);
      return;
    }
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

  // The zoom menu: Fit and a few fixed levels. A level at or below Fit would only fit, so it's off.
  const ZOOMS: { label: string; zoom: number | null; keys?: string }[] = [
    { label: "Fit", zoom: null, keys: "Ctrl+0" },
    { label: "50%", zoom: 0.5 },
    { label: "100%", zoom: 1, keys: "Ctrl+1" },
    { label: "200%", zoom: 2 },
  ];
  let zoomOpen = $state(false);
  let zoomMenu = $state<HTMLDivElement>();
  let fitZoom = $state(1);
  function toggleZoomMenu() {
    if (!zoomOpen && renderer) fitZoom = renderer.fitScale();
    zoomOpen = !zoomOpen;
  }
  function pickZoom(z: number | null) {
    zoomOpen = false;
    setZoom(z);
  }
  function onWindowDown(e: PointerEvent) {
    if (zoomOpen && zoomMenu && !zoomMenu.contains(e.target as Node)) zoomOpen = false;
  }
  function onMenuKey(e: KeyboardEvent) {
    if (zoomOpen && e.key === "Escape") {
      e.stopPropagation();
      zoomOpen = false;
    }
  }
  $effect(() => {
    if (cropping) zoomOpen = false;
  });

  // ---- pixel readout ----
  // The displayed RGB under the pointer: one pixel rendered and read back, at most once a frame.
  // Hidden while a button is down (panning, cropping, painting a mask) and off the photo.
  let pixel = $state<[number, number, number] | null>(null);
  let hoverAt: readonly [number, number] | null = null;
  let probeFrame = 0;
  function probe() {
    if (!hoverAt || probeFrame) return;
    probeFrame = requestAnimationFrame(() => {
      probeFrame = 0;
      pixel = hoverAt && renderer ? renderer.readPixel(hoverAt[0], hoverAt[1]) : null;
    });
  }
  function onHover(e: PointerEvent) {
    const onChrome = e.target instanceof Element && e.target.closest(".zoom, .nav");
    if (e.buttons !== 0 || onChrome) return endHover();
    hoverAt = local(e);
    probe();
  }
  function endHover() {
    hoverAt = null;
    pixel = null;
  }
  onMount(() => () => cancelAnimationFrame(probeFrame));

  /** Bumped when the main view's picture changes, so the navigator redraws its overview. */
  let drawEpoch = $state(0);

  // Latest-wins parameter pipelines (the main view, and the compare pane): at most one request
  // in flight each, always ending on the newest state. The curve LUT is only refetched when the
  // curve itself changes.
  function pipeline(
    apply: (
      r: PreviewRenderer,
      params: Float32Array,
      curve: Uint16Array | null,
      brush: BrushLayers | null | undefined,
    ) => void,
  ) {
    let inflight = false;
    // Each request carries its own renderer and photo: switching photos mid-flight must not
    // develop the new edit against the old photo's colour, lens and size.
    type Job = { r: PreviewRenderer; id: number; state: EditState };
    let pending: Job | null = null;
    let curveKey = "";
    let brushK: string | null = null;
    let lastR: PreviewRenderer | null = null;
    async function pump() {
      if (inflight) return;
      inflight = true;
      try {
        while (pending) {
          const { r, id, state: next } = pending;
          pending = null;
          // A new renderer (context restored) has none of the cached inputs.
          if (r !== lastR) {
            lastR = r;
            curveKey = "";
            brushK = null;
          }
          const key = JSON.stringify(next.curve);
          // Brush rasters only when strokes change (undefined: unchanged; null: none).
          const layersKey = brushKey(next, session.aiEpoch);
          const bk = `${id}:${layersKey}`;
          const [params, curve, brush, maps] = await Promise.all([
            // Params don't depend on stroke points: don't serialise them on every tick.
            api.developParams(id, withoutStrokes(next)),
            key !== curveKey ? api.developCurve(next) : null,
            bk === brushK ? undefined : layersKey ? api.maskLayers(id, next) : null,
            usesPresenceMaps(next) && !r.hasPresence(id) ? api.presenceMaps(id) : null,
          ]);
          // Superseded by another photo (or renderer) meanwhile: its own request follows.
          const later = pending as Job | null; // (set by calls made during the await)
          if (later && (later.id !== id || later.r !== r)) continue;
          if (curve) curveKey = key;
          brushK = bk;
          if (maps) r.setPresence(id, maps);
          apply(r, params, curve, brush);
          if (renderError !== null) toasts.dismiss(renderError);
          renderError = null;
        }
      } catch (e) {
        renderError = toasts.error("Couldn’t draw the preview", e);
      } finally {
        inflight = false;
        // A failed request leaves a newer one queued: run it.
        if (pending) pump();
      }
    }
    const push = (r: PreviewRenderer, id: number, state: EditState) => {
      pending = { r, id, state };
      pump();
    };
    /** The mask rasters the renderer holds are no longer what was fetched: fetch them again
     *  with the next request, though the strokes haven't changed. */
    push.forgetBrush = () => {
      brushK = null;
    };
    return push;
  }
  const main = pipeline((r, params, curve, brush) => {
    if (curve) r.setCurve(curve);
    if (brush !== undefined) r.setBrush("main", brush);
    if (brush?.missing?.length) session.detectMissing(brush.missing);
    r.setParams(params);
    session.framing = r.framing;
    refreshMaskThumbs();
    drawEpoch++;
    probe();
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
    // Hovering a mask in the list shows its area even with the overlay off.
    const hover = session.masking && !before ? session.maskHover : null;
    if (hover !== null && hover < masks.length) {
      r.setOverlay(shaderIndex(masks, hover));
      return;
    }
    const show = selected !== null && (session.maskOverlay || interacting);
    r.setOverlay(show && selected !== null ? shaderIndex(masks, selected) : null);
  });

  // Black-and-white mask thumbnails for the mask list, redrawn shortly after the params settle.
  let thumbTimer: ReturnType<typeof setTimeout> | undefined;
  function refreshMaskThumbs() {
    clearTimeout(thumbTimer);
    if (!session.masking) return;
    thumbTimer = setTimeout(() => {
      const r = renderer;
      if (!r || !session.masking || before) return;
      const ms = masksOf(edit);
      const idx = ms.map((_, i) => shaderIndex(ms, i));
      const drawn =
        r.renderMasks(
          idx.filter((v) => v !== null),
          MASK_THUMB_EDGE,
        ) ?? [];
      let k = 0;
      session.maskThumbs = idx.map((v) => (v === null ? null : (drawn[k++] ?? null)));
    }, 120);
  }
  $effect(() => {
    if (session.masking && renderer) untrack(refreshMaskThumbs);
  });
  onMount(() => () => clearTimeout(thumbTimer));

  function setMask(i: number, m: Mask) {
    if (!edit.masks) return;
    edit.masks[i] = m;
  }

  function paintLive(i: number, stroke: Stroke, a: readonly number[], b: readonly number[]) {
    const r = renderer;
    if (!r) return;
    r.paintLive(brushLayer(masks, i), stroke, a, b, maskSize(image.fullWidth, image.fullHeight));
  }

  /** Bumped to draw the main view again though nothing in the edit changed. */
  let redraw = $state(0);

  /**
   * A stroke was abandoned in the middle (Esc, M or \ with the pointer still down): it was
   * painted into the preview's mask but is in no edit. Take it out, or the photo on screen
   * differs from what is saved and exported, and the renderer keeps painting it back in.
   */
  function cancelStroke() {
    renderer?.endLive();
    main.forgetBrush();
    redraw++;
  }

  function endStroke(i: number, stroke: Stroke) {
    renderer?.endLive();
    const m = edit.masks?.[i];
    if (m?.shape.kind === "brush") m.shape.strokes.push(stroke);
    else if (m?.shape.kind === "ai") m.shape.refine.push(stroke);
    else return;
    session.commit(null);
  }

  function onKey(e: KeyboardEvent) {
    // Space on a button reached with Tab presses the button; anywhere else in mask mode,
    // held, it pans.
    if (e.key === " " && session.masking && !focusedControlOwnsKey(e.target, " ")) {
      spaceHeld = e.type === "keydown";
      if (spaceHeld) e.preventDefault();
    }
  }

  $effect(() => {
    const state = $state.snapshot(before ? beforeEdit : edit) as EditState;
    // While cropping, show the whole (straightened) frame; the overlay shows the crop.
    if (cropping && !before) state.crop = { ...state.crop, rect: [...FULL] };
    const r = renderer;
    void session.aiEpoch; // a finished detection means new layers
    void redraw;
    if (r) main(r, image.id, state);
  });

  $effect(() => {
    if (cropping) renderer?.setZoom(null);
  });

  // Another photo in the compare pane (before its params, below).
  const other = $derived(compare?.image && compare.pixels ? compare.image : null);
  $effect(() => {
    const r = renderer;
    const o = other;
    const px = compare?.pixels;
    if (!r) return;
    r.setCompareImage(
      o && px
        ? {
            id: o.id,
            width: o.width,
            height: o.height,
            fullW: o.fullWidth,
            fullH: o.fullHeight,
            pixels: px,
          }
        : null,
    );
  });

  let comparing = false;
  $effect(() => {
    const r = renderer;
    const state = compare ? ($state.snapshot(compare.state) as EditState) : null;
    const id = other?.id ?? image.id;
    if (!r) return;
    comparing = state !== null;
    if (state) side(r, id, state);
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

<svelte:window
  onkeydown={onKey}
  onkeyup={onKey}
  onblur={() => (spaceHeld = false)}
  onpointerdown={onWindowDown}
  onkeydowncapture={onMenuKey}
/>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="wrap"
  onwheel={onWheel}
  onpointermove={onHover}
  onpointerdown={() => (pixel = null)}
  onpointerleave={endHover}
>
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <canvas
    bind:this={canvas}
    class:zoomed={!fit}
    class:panning
    class:picking={!!onpick}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
  ></canvas>
  {#if cropping && oncrop && frameRect.w > 0}
    <CropOverlay
      crop={edit.crop}
      frame={frameRect}
      tw={turned[0]}
      th={turned[1]}
      aspect={cropAspect}
      {level}
      onchange={oncrop}
      {onlevel}
    />
  {/if}
  {#if session.lassoing && !cropping && renderer && frameRect.w > 0}
    <LassoOverlay
      frame={frameRect}
      framing={session.framing ?? renderer.framing}
      busy={!!session.lassoing.busy}
      ondone={(lasso) => session.finishLasso(lasso)}
      oncancel={() => session.cancelLasso()}
    />
  {:else if selected !== null && masks[selected] && renderer && frameRect.w > 0}
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
      oncancel={cancelStroke}
      oninteract={(v) => (interacting = v)}
    />
  {/if}
  <div class="zoom" class:hidden={cropping}>
    {#if loadingDetail}<span class="loading">Loading detail…</span>{/if}
    <div class="zoom-menu" bind:this={zoomMenu}>
      <button
        class="trigger"
        class:open={zoomOpen}
        onclick={toggleZoomMenu}
        title="Zoom (Z toggles Fit / 100%)"
        aria-haspopup="menu"
        aria-expanded={zoomOpen}
      >
        {zoomLabel}<span class="caret">▾</span>
      </button>
      {#if zoomOpen}
        <div class="pop" role="menu">
          {#each ZOOMS as z (z.label)}
            {@const current = z.zoom === null ? fit : !fit && Math.abs(scale - z.zoom) < 1e-3}
            <button
              role="menuitemradio"
              aria-checked={current}
              class:current
              disabled={z.zoom !== null && z.zoom <= fitZoom + 1e-6}
              onclick={() => pickZoom(z.zoom)}
            >
              <span>{z.label}</span>
              {#if z.keys}<kbd>{z.keys}</kbd>{/if}
            </button>
          {/each}
        </div>
      {/if}
    </div>
  </div>
  {#if renderer && !fit && !cropping && frameRect.w > 0}
    <Navigator {renderer} frame={frameRect} epoch={drawEpoch} />
  {/if}
  {#if pixel}
    <div class="readout" aria-hidden="true">
      <span>R</span>{pixel[0]} <span>G</span>{pixel[1]} <span>B</span>{pixel[2]}
    </div>
  {/if}
  {#snippet closeCompare()}
    {#if oncloseCompare}<button onclick={oncloseCompare} title="Stop comparing (Esc)">
        ×
      </button>{/if}
  {/snippet}
  {#if compare?.image}
    <!-- Another photo: it's on the left, the one being edited on the right. -->
    <div class="pane-label left" title={compare.label}>
      {compare.label}
      {@render closeCompare()}
    </div>
    <div class="pane-label right">{before ? "Before" : compare.current}</div>
  {:else if compare}
    <div class="pane-label left">{before ? "Before" : compare.current}</div>
    <div class="pane-label right">
      {compare.label}
      {@render closeCompare()}
    </div>
  {:else if before}<div class="badge">Before</div>{/if}
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
  canvas.picking {
    cursor: crosshair;
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
  .zoom-menu {
    position: relative;
  }
  .trigger {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 4px;
    min-width: 52px;
    padding: 2px 8px;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.45);
    color: var(--text-1);
    font-family: var(--font-num);
  }
  .trigger:hover,
  .trigger.open {
    color: var(--text-0);
  }
  .caret {
    font-size: 8px;
    color: var(--text-2);
  }
  .pop {
    position: absolute;
    top: calc(100% + 4px);
    right: 0;
    z-index: 40;
    min-width: 132px;
    padding: var(--space-1);
    border-radius: var(--radius-m);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
  }
  .pop button {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    width: 100%;
    padding: 4px var(--space-2);
    border-radius: var(--radius-s);
    color: var(--text-1);
    font-family: var(--font-num);
    text-align: left;
  }
  .pop button:hover:not(:disabled),
  .pop button.current {
    background: var(--bg-2);
    color: var(--text-0);
  }
  .pop button:disabled {
    color: var(--text-2);
    opacity: 0.5;
  }
  .pop kbd {
    font-family: inherit;
    color: var(--text-2);
  }
  .loading {
    color: var(--text-2);
  }
  .readout {
    position: absolute;
    left: var(--space-3);
    bottom: var(--space-3);
    padding: 1px 8px;
    border-radius: 999px;
    background: rgba(0, 0, 0, 0.45);
    font-family: var(--font-num);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
    color: var(--text-1);
    white-space: pre;
    pointer-events: none;
  }
  .readout span {
    color: var(--text-2);
    margin-right: 3px;
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
</style>
