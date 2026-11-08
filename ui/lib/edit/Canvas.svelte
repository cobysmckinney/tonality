<script lang="ts">
  import { onMount } from "svelte";
  import { api } from "$lib/api";
  import type { EditState } from "$lib/bindings/EditState";
  import type { ImageInfo } from "$lib/bindings/ImageInfo";
  import { PreviewRenderer, type Histogram } from "./renderer";

  interface Props {
    image: ImageInfo;
    pixels: Uint16Array;
    edit: EditState;
    /** Show the unedited image (before/after). */
    before?: boolean;
    beforeEdit: EditState;
    onhistogram?: (h: Histogram) => void;
  }
  let { image, pixels, edit, before = false, beforeEdit, onhistogram }: Props = $props();

  let canvas: HTMLCanvasElement;
  let renderer = $state<PreviewRenderer | null>(null);
  let error = $state<string | null>(null);

  onMount(() => {
    let disposed = false;
    const ro = new ResizeObserver(() => renderer?.requestDraw());
    ro.observe(canvas);
    api
      .previewShaders()
      .then(async (src) => {
        if (disposed) return;
        const r = new PreviewRenderer(canvas, src);
        if (await api.devSnapshotPath()) {
          r.onDrawn = (c) => {
            r.onDrawn = null;
            api.devSaveSnapshot(c.toDataURL("image/png"));
          };
        }
        r.onHistogram = (h) => onhistogram?.(h);
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
    renderer?.setImage(image.width, image.height, pixels);
  });

  // Latest-wins parameter pipeline: at most one request in flight, always ending on the newest state.
  let inflight = false;
  let pending: EditState | null = null;
  // The curve LUT only needs refetching when the curve itself changes.
  let curveKey = JSON.stringify({ rgb: [], red: [], green: [], blue: [] });
  async function pump(r: PreviewRenderer, id: number) {
    if (inflight) return;
    inflight = true;
    try {
      while (pending) {
        const next = pending;
        pending = null;
        const key = JSON.stringify(next.curve);
        const [params, curve] = await Promise.all([
          api.developParams(id, next),
          key !== curveKey ? api.developCurve(next) : null,
        ]);
        if (curve) {
          r.setCurve(curve);
          curveKey = key;
        }
        r.setParams(params);
      }
    } catch (e) {
      error = String(e);
    } finally {
      inflight = false;
    }
  }

  $effect(() => {
    const state = $state.snapshot(before ? beforeEdit : edit) as EditState;
    const r = renderer;
    const id = image.id;
    if (!r) return;
    pending = state;
    pump(r, id);
  });
</script>

<div class="wrap">
  <canvas bind:this={canvas}></canvas>
  {#if before}<div class="badge">Before</div>{/if}
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
