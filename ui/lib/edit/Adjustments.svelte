<script lang="ts">
  import type { ColorMixer } from "$lib/bindings/ColorMixer";
  import type { EditState } from "$lib/bindings/EditState";
  import type { HueBand } from "$lib/bindings/HueBand";
  import type { ImageInfo } from "$lib/bindings/ImageInfo";
  import Section from "$lib/design/Section.svelte";
  import Slider from "$lib/design/Slider.svelte";
  import CurveEditor from "./CurveEditor.svelte";
  import { session } from "./session.svelte";
  import HistogramView from "./Histogram.svelte";
  import type { Histogram } from "./renderer";

  interface Props {
    edit: EditState;
    image: ImageInfo;
    histogram: Histogram | null;
    /**
     * Called once per completed gesture. `key` names the control so the history can merge
     * rapid repeats (arrow-key nudges); `null` for one-off actions.
     */
    oncommit?: (key: string | null) => void;
  }
  let { edit = $bindable(), image, histogram, oncommit }: Props = $props();

  type LightKey = keyof EditState["light"];
  const lightControls: { key: LightKey; label: string; min: number; max: number; step: number; decimals: number }[] = [
    { key: "exposure", label: "Exposure", min: -5, max: 5, step: 0.01, decimals: 2 },
    { key: "contrast", label: "Contrast", min: -100, max: 100, step: 1, decimals: 0 },
    { key: "highlights", label: "Highlights", min: -100, max: 100, step: 1, decimals: 0 },
    { key: "shadows", label: "Shadows", min: -100, max: 100, step: 1, decimals: 0 },
    { key: "whites", label: "Whites", min: -100, max: 100, step: 1, decimals: 0 },
    { key: "blacks", label: "Blacks", min: -100, max: 100, step: 1, decimals: 0 },
  ];

  const wb = $derived(edit.whiteBalance);
  const isAsShot = $derived(wb.temperature === null && wb.tint === null);

  function resetWb() {
    edit.whiteBalance = { temperature: null, tint: null };
    oncommit?.(null);
  }

  // ---- colour mixer ----
  type Band = keyof ColorMixer;
  type BandProp = keyof HueBand;
  const bands: { id: Band; label: string; hue: number }[] = [
    { id: "red", label: "Red", hue: 0 },
    { id: "orange", label: "Orange", hue: 30 },
    { id: "yellow", label: "Yellow", hue: 55 },
    { id: "green", label: "Green", hue: 120 },
    { id: "aqua", label: "Aqua", hue: 180 },
    { id: "blue", label: "Blue", hue: 225 },
    { id: "purple", label: "Purple", hue: 270 },
    { id: "magenta", label: "Magenta", hue: 310 },
  ];
  const mixerModes: { id: BandProp; label: string }[] = [
    { id: "hue", label: "Hue" },
    { id: "saturation", label: "Saturation" },
    { id: "luminance", label: "Luminance" },
  ];
  let mixerMode = $state<BandProp>("hue");

  function bandTrack(i: number, mode: BandProp): string {
    const h = bands[i].hue;
    if (mode === "hue") {
      const prev = bands[(i + 7) % 8].hue;
      const next = bands[(i + 1) % 8].hue;
      const mid = (a: number, b: number) => (b < a ? (a + b + 360) / 2 : (a + b) / 2) % 360;
      return `linear-gradient(90deg, hsl(${mid(prev, h)} 70% 55%), hsl(${h} 70% 55%), hsl(${mid(h, next)} 70% 55%))`;
    }
    if (mode === "saturation") return `linear-gradient(90deg, hsl(${h} 0% 55%), hsl(${h} 85% 55%))`;
    return `linear-gradient(90deg, hsl(${h} 70% 18%), hsl(${h} 70% 55%), hsl(${h} 70% 85%))`;
  }

  const mixerEdited = $derived(
    bands.some((b) => {
      const v = edit.color.mixer[b.id];
      return v.hue || v.saturation || v.luminance;
    }),
  );
  function resetMixer() {
    for (const b of bands) edit.color.mixer[b.id] = { hue: 0, saturation: 0, luminance: 0 };
    oncommit?.(null);
  }
</script>

<div class="panel">
  <HistogramView {histogram} />

  <div class="scroll">
    <div class="tools">
      <button class="crop" onclick={() => session.startCrop()} title="Crop, rotate and straighten (R)">
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M4 1v11h11M1 4h11v11" /></svg>
        Crop &amp; Rotate
      </button>
      <button class="crop" onclick={() => session.startMasks()} title="Local adjustments: gradients and brush (M)">
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><circle cx="8" cy="8" r="6" /><path d="M8 2a6 6 0 0 0 0 12Z" /></svg>
        Masks{#if edit.masks?.length}<span class="count">{edit.masks.length}</span>{/if}
      </button>
    </div>
    <Section title="White Balance">
      <div class="wb-mode">
        <button class:active={isAsShot} onclick={resetWb} disabled={isAsShot}>As Shot</button>
        <span class="hint">{isAsShot ? "" : "Custom"}</span>
      </div>
      <Slider
        label="Temperature"
        value={wb.temperature ?? image.asShotTemperature}
        min={2000}
        max={12000}
        step={50}
        defaultValue={image.asShotTemperature}
        unit="K"
        bipolar={false}
        track="linear-gradient(90deg, #4f7fd9, #d8d8d8 40%, #e0a63c)"
        oninput={(v) => (edit.whiteBalance.temperature = v)}
        oncommit={() => oncommit?.("temperature")}
        onreset={() => {
          edit.whiteBalance.temperature = null;
          oncommit?.(null);
        }}
      />
      <Slider
        label="Tint"
        value={wb.tint ?? image.asShotTint}
        min={-150}
        max={150}
        defaultValue={image.asShotTint}
        track="linear-gradient(90deg, #4fb55a, #d8d8d8 50%, #c45bc4)"
        oninput={(v) => (edit.whiteBalance.tint = v)}
        oncommit={() => oncommit?.("tint")}
        onreset={() => {
          edit.whiteBalance.tint = null;
          oncommit?.(null);
        }}
      />
    </Section>

    <Section title="Light">
      {#each lightControls as c (c.key)}
        <Slider
          label={c.label}
          value={edit.light[c.key]}
          min={c.min}
          max={c.max}
          step={c.step}
          decimals={c.decimals}
          oninput={(v) => (edit.light[c.key] = v)}
          oncommit={() => oncommit?.(c.key)}
        />
      {/each}
    </Section>

    <Section title="Tone Curve">
      <CurveEditor
        value={edit.curve}
        {histogram}
        oninput={(c) => (edit.curve = c)}
        oncommit={() => oncommit?.("curve")}
      />
    </Section>

    <Section title="Color">
      <Slider
        label="Vibrance"
        value={edit.color.vibrance}
        min={-100}
        max={100}
        oninput={(v) => (edit.color.vibrance = v)}
        oncommit={() => oncommit?.("vibrance")}
      />
      <Slider
        label="Saturation"
        value={edit.color.saturation}
        min={-100}
        max={100}
        oninput={(v) => (edit.color.saturation = v)}
        oncommit={() => oncommit?.("saturation")}
      />

      <div class="mixer-head">
        <span class="sub">Color Mixer</span>
        <button class="link" onclick={resetMixer} disabled={!mixerEdited}>Reset</button>
      </div>
      <div class="seg" role="tablist">
        {#each mixerModes as m (m.id)}
          <button role="tab" aria-selected={mixerMode === m.id} class:active={mixerMode === m.id} onclick={() => (mixerMode = m.id)}>
            {m.label}
          </button>
        {/each}
      </div>
      {#each bands as b, i (b.id)}
        <Slider
          label={b.label}
          value={edit.color.mixer[b.id][mixerMode]}
          min={-100}
          max={100}
          track={bandTrack(i, mixerMode)}
          oninput={(v) => (edit.color.mixer[b.id][mixerMode] = v)}
          oncommit={() => oncommit?.(`mixer.${b.id}.${mixerMode}`)}
        />
      {/each}
    </Section>

    <Section title="Detail">
      <p class="note">Sharpening and noise reduction are sized for full resolution. Zoom to 100% (Z) to judge them.</p>
      <span class="sub">Sharpening</span>
      <Slider
        label="Amount"
        value={edit.detail.sharpenAmount}
        min={0}
        max={150}
        bipolar={false}
        oninput={(v) => (edit.detail.sharpenAmount = v)}
        oncommit={() => oncommit?.("detail.sharpenAmount")}
      />
      <Slider
        label="Radius"
        value={edit.detail.sharpenRadius}
        min={0.5}
        max={3}
        step={0.1}
        decimals={1}
        defaultValue={1}
        bipolar={false}
        oninput={(v) => (edit.detail.sharpenRadius = v)}
        oncommit={() => oncommit?.("detail.sharpenRadius")}
      />
      <Slider
        label="Masking"
        value={edit.detail.sharpenMasking}
        min={0}
        max={100}
        bipolar={false}
        oninput={(v) => (edit.detail.sharpenMasking = v)}
        oncommit={() => oncommit?.("detail.sharpenMasking")}
      />
      <span class="sub">Noise Reduction</span>
      <Slider
        label="Luminance"
        value={edit.detail.luminanceNoise}
        min={0}
        max={100}
        bipolar={false}
        oninput={(v) => (edit.detail.luminanceNoise = v)}
        oncommit={() => oncommit?.("detail.luminanceNoise")}
      />
      <Slider
        label="Color"
        value={edit.detail.colorNoise}
        min={0}
        max={100}
        bipolar={false}
        oninput={(v) => (edit.detail.colorNoise = v)}
        oncommit={() => oncommit?.("detail.colorNoise")}
      />
    </Section>

    <Section title="Effects">
      <span class="sub">Vignette</span>
      <Slider
        label="Amount"
        value={edit.effects.vignette.amount}
        min={-100}
        max={100}
        oninput={(v) => (edit.effects.vignette.amount = v)}
        oncommit={() => oncommit?.("vignette.amount")}
      />
      <Slider
        label="Midpoint"
        value={edit.effects.vignette.midpoint}
        min={0}
        max={100}
        defaultValue={50}
        bipolar={false}
        oninput={(v) => (edit.effects.vignette.midpoint = v)}
        oncommit={() => oncommit?.("vignette.midpoint")}
      />
      <Slider
        label="Feather"
        value={edit.effects.vignette.feather}
        min={0}
        max={100}
        defaultValue={50}
        bipolar={false}
        oninput={(v) => (edit.effects.vignette.feather = v)}
        oncommit={() => oncommit?.("vignette.feather")}
      />
      <span class="sub">Grain</span>
      <Slider
        label="Amount"
        value={edit.effects.grain.amount}
        min={0}
        max={100}
        bipolar={false}
        oninput={(v) => (edit.effects.grain.amount = v)}
        oncommit={() => oncommit?.("grain.amount")}
      />
      <Slider
        label="Size"
        value={edit.effects.grain.size}
        min={0}
        max={100}
        defaultValue={25}
        bipolar={false}
        oninput={(v) => (edit.effects.grain.size = v)}
        oncommit={() => oncommit?.("grain.size")}
      />
    </Section>
  </div>
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
  }
  .tools {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4) 0;
  }
  .count {
    margin-left: auto;
    font-family: var(--font-num);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .crop {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 6px 10px;
    border-radius: var(--radius-m);
    background: var(--bg-2);
    color: var(--text-1);
    font-size: var(--fs-s);
  }
  .crop:hover {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .crop svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
  }
  .wb-mode {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }
  .wb-mode button {
    font-size: var(--fs-xs);
    padding: 2px 8px;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    color: var(--text-1);
  }
  .wb-mode button.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .wb-mode button:disabled {
    cursor: default;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .sub {
    display: block;
    margin: var(--space-3) 0 var(--space-1);
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-2);
  }
  .note {
    margin: 0 0 var(--space-1);
    font-size: var(--fs-xs);
    color: var(--text-2);
    line-height: 1.4;
  }
  .mixer-head {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-top: var(--space-2);
  }
  .link {
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .link:hover:not(:disabled) {
    color: var(--text-0);
  }
  .link:disabled {
    opacity: 0.35;
  }
  .seg {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: 2px;
    padding: 2px;
    margin-bottom: var(--space-2);
    border-radius: var(--radius-m);
    background: var(--bg-0);
  }
  .seg button {
    padding: 3px 0;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .seg button.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
</style>
