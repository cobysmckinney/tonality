<script lang="ts">
  // RGB histogram of the rendered image, with shadow/highlight clipping indicators.
  import type { Histogram } from "./renderer";

  interface Props {
    histogram: Histogram | null;
  }
  let { histogram }: Props = $props();

  const H = 84;
  let canvas: HTMLCanvasElement;
  let width = $state(0);

  /** Share of pixels with any channel at 0 / 255. */
  const clip = $derived.by(() => {
    if (!histogram) return { lo: 0, hi: 0 };
    const t = histogram.total || 1;
    return {
      lo: Math.max(histogram.r[0], histogram.g[0], histogram.b[0]) / t,
      hi: Math.max(histogram.r[255], histogram.g[255], histogram.b[255]) / t,
    };
  });

  $effect(() => {
    const h = histogram;
    const w = width;
    if (!canvas || !w) return;
    const dpr = window.devicePixelRatio || 1;
    canvas.width = Math.round(w * dpr);
    canvas.height = Math.round(H * dpr);
    const ctx = canvas.getContext("2d")!;
    ctx.setTransform(dpr, 0, 0, dpr, 0, 0);
    ctx.clearRect(0, 0, w, H);
    if (!h) return;

    // Scale to the tallest interior bin so clipped spikes at 0/255 don't flatten everything.
    let peak = 1;
    for (const ch of [h.r, h.g, h.b]) for (let i = 1; i < 255; i++) peak = Math.max(peak, ch[i]);
    const y = (v: number) => H - Math.min(1, Math.sqrt(v / peak) * 0.96) * H;

    ctx.globalCompositeOperation = "lighter";
    const channels: [Uint32Array, string][] = [
      [h.r, "rgba(220, 70, 70, 0.75)"],
      [h.g, "rgba(70, 200, 90, 0.75)"],
      [h.b, "rgba(80, 120, 235, 0.75)"],
    ];
    for (const [ch, colour] of channels) {
      ctx.beginPath();
      ctx.moveTo(0, H);
      for (let i = 0; i < 256; i++) ctx.lineTo((i / 255) * w, y(ch[i]));
      ctx.lineTo(w, H);
      ctx.closePath();
      ctx.fillStyle = colour;
      ctx.fill();
    }
    ctx.globalCompositeOperation = "source-over";
  });

  const pct = (v: number) => (v < 0.001 ? "" : `${(v * 100).toFixed(v < 0.01 ? 1 : 0)}%`);
</script>

<div class="hist" bind:clientWidth={width}>
  <canvas bind:this={canvas} style:height="{H}px"></canvas>
  <span class="clip lo" class:on={clip.lo >= 0.001} title="Shadows clipped {pct(clip.lo) || '0%'}"></span>
  <span class="clip hi" class:on={clip.hi >= 0.001} title="Highlights clipped {pct(clip.hi) || '0%'}"></span>
</div>

<style>
  .hist {
    position: relative;
    margin: var(--space-3) var(--space-4) var(--space-2);
    border-radius: var(--radius-s);
    background: var(--bg-0);
    overflow: hidden;
  }
  canvas {
    display: block;
    width: 100%;
  }
  .clip {
    position: absolute;
    top: 4px;
    width: 0;
    height: 0;
    border-style: solid;
    opacity: 0.25;
  }
  .clip.on {
    opacity: 1;
  }
  .lo {
    left: 4px;
    border-width: 6px 6px 0 0;
    border-color: var(--text-1) transparent transparent transparent;
  }
  .hi {
    right: 4px;
    border-width: 0 6px 6px 0;
    border-color: transparent var(--text-1) transparent transparent;
  }
</style>
