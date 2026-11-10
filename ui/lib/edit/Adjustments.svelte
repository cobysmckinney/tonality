<script lang="ts">
  import type { ColorMixer } from "$lib/bindings/ColorMixer";
  import type { EditState } from "$lib/bindings/EditState";
  import type { HueBand } from "$lib/bindings/HueBand";
  import type { LensCorrection } from "$lib/bindings/LensCorrection";
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
  // ---- lens ----
  // `edit.lens` is absent until first touched (so untouched states keep their hashes).
  const LENS_DEFAULT: LensCorrection = { profile: false, distortion: 100, vignetting: 100, chromatic: true, manualDistortion: 0 };
  const lens = $derived(edit.lens ?? LENS_DEFAULT);
  function setLens(patch: Partial<LensCorrection>) {
    edit.lens = { ...(edit.lens ?? LENS_DEFAULT), ...patch };
  }
  const lensInfo = $derived(image.lens);

  function resetMixer() {
    for (const b of bands) edit.color.mixer[b.id] = { hue: 0, saturation: 0, luminance: 0 };
    oncommit?.(null);
  }

  // ---- per-section "edited" state and reset ----
  let defaults = $state.raw<EditState | null>(null);
  $effect(() => {
    session.defaultEdit().then((d) => (defaults = d));
  });
  const same = (a: unknown, b: unknown) => JSON.stringify(a) === JSON.stringify(b);
  const edited = $derived({
    wb: !isAsShot,
    light: !!defaults && !same(edit.light, defaults.light),
    curve: !!defaults && !same(edit.curve, defaults.curve),
    color: edit.color.vibrance !== 0 || edit.color.saturation !== 0,
    mixer: mixerEdited,
    detail: !!defaults && !same(edit.detail, defaults.detail),
    lens: !!edit.lens && !same(edit.lens, LENS_DEFAULT),
    effects: !!defaults && !same(edit.effects, defaults.effects),
  });

  function reset(section: "light" | "curve" | "detail" | "effects") {
    if (!defaults) return;
    (edit as Record<string, unknown>)[section] = structuredClone($state.snapshot(defaults[section]));
    oncommit?.(null);
  }
  function resetColor() {
    edit.color.vibrance = 0;
    edit.color.saturation = 0;
    oncommit?.(null);
  }
  function resetLens() {
    edit.lens = { ...LENS_DEFAULT };
    oncommit?.(null);
  }
</script>

<div class="panel">
  <div class="top">
    <HistogramView {histogram} />
    <div class="tools">
      <button class="tool" onclick={() => session.startCrop()} title="Crop, rotate and straighten (R)">
        <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"><path d="M4 1v11h11M1 4h11v11" /></svg>
        <span>Crop</span>
        <kbd>R</kbd>
      </button>
      <button class="tool" onclick={() => session.startMasks()} title="Local adjustments: gradients and brush (M)">
        <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"><circle cx="8" cy="8" r="6" /><path d="M8 2a6 6 0 0 0 0 12Z" /></svg>
        <span>Masks</span>
        {#if edit.masks?.length}<span class="count">{edit.masks.length}</span>{:else}<kbd>M</kbd>{/if}
      </button>
    </div>
  </div>

  <div class="scroll">
    <Section title="White Balance" edited={edited.wb} onreset={resetWb}>
      <div class="seg wb-mode">
        <button class:active={isAsShot} onclick={resetWb}>As Shot</button>
        <button class:active={!isAsShot} disabled={isAsShot} title="Move a slider to set a custom balance">Custom</button>
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

    <Section title="Light" edited={edited.light} onreset={() => reset("light")}>
      {#each lightControls as c, i (c.key)}
        {#if i === 2}<div class="spacer"></div>{/if}
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

    <Section title="Tone Curve" edited={edited.curve} onreset={() => reset("curve")}>
      <CurveEditor
        value={edit.curve}
        {histogram}
        oninput={(c) => (edit.curve = c)}
        oncommit={() => oncommit?.("curve")}
      />
    </Section>

    <Section title="Color" edited={edited.color} onreset={resetColor}>
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
    </Section>

    <Section title="Color Mixer" open={false} edited={edited.mixer} onreset={resetMixer}>
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

    <Section title="Detail" open={false} edited={edited.detail} onreset={() => reset("detail")}>
      <p class="note">Sized for full resolution. Zoom to 100% (Z) to judge them.</p>
      <div class="group">
        <span class="eyebrow">Sharpening</span>
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
      </div>
      <div class="group">
        <span class="eyebrow">Noise Reduction</span>
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
      </div>
    </Section>

    <Section title="Lens Correction" open={false} edited={edited.lens} onreset={resetLens}>
      <div class="group">
        <span class="eyebrow">Profile</span>
        {#if lensInfo}
          <label class="check">
            <input
              type="checkbox"
              checked={lens.profile}
              onchange={(e) => {
                setLens({ profile: e.currentTarget.checked });
                oncommit?.(null);
              }}
            />
            Enable profile corrections
          </label>
          <p class="note">
            {lensInfo.name} · {lensInfo.source === "embedded" ? "the camera's own correction data" : "lensfun database"}
          </p>
          {#if lens.profile}
            {#if lensInfo.distortion}
              <Slider
                label="Distortion"
                value={lens.distortion}
                min={0}
                max={200}
                defaultValue={100}
                bipolar={false}
                unit="%"
                oninput={(v) => setLens({ distortion: v })}
                oncommit={() => oncommit?.("lens.distortion")}
              />
            {/if}
            {#if lensInfo.vignetting}
              <Slider
                label="Vignetting"
                value={lens.vignetting}
                min={0}
                max={200}
                defaultValue={100}
                bipolar={false}
                unit="%"
                oninput={(v) => setLens({ vignetting: v })}
                oncommit={() => oncommit?.("lens.vignetting")}
              />
            {/if}
            {#if lensInfo.chromatic}
              <label class="check">
                <input
                  type="checkbox"
                  checked={lens.chromatic}
                  onchange={(e) => {
                    setLens({ chromatic: e.currentTarget.checked });
                    oncommit?.(null);
                  }}
                />
                Remove chromatic aberration
              </label>
            {/if}
          {/if}
        {:else}
          <p class="note">No correction profile for this lens. Manual distortion still works.</p>
        {/if}
      </div>
      <div class="group">
        <span class="eyebrow">Manual</span>
        <Slider
          label="Distortion"
          value={lens.manualDistortion}
          min={-100}
          max={100}
          oninput={(v) => setLens({ manualDistortion: v })}
          oncommit={() => oncommit?.("lens.manualDistortion")}
        />
      </div>
    </Section>

    <Section title="Effects" open={false} edited={edited.effects} onreset={() => reset("effects")}>
      <div class="group">
        <span class="eyebrow">Vignette</span>
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
      </div>
      <div class="group">
        <span class="eyebrow">Grain</span>
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
      </div>
    </Section>
    <div class="end"></div>
  </div>
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  .top {
    flex: none;
    padding-bottom: var(--space-3);
    border-bottom: 1px solid var(--line);
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-top: var(--space-2);
  }
  .end {
    height: var(--space-5);
  }
  .tools {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
    padding: 0 var(--space-4);
  }
  .tool {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 34px;
    padding: 0 var(--space-3);
    border-radius: var(--radius-m);
    background: var(--bg-2);
    color: var(--text-1);
    font-size: var(--fs-s);
    transition: background 120ms var(--ease), color 120ms var(--ease);
  }
  .tool:hover {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .tool svg {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
  }
  kbd,
  .count {
    margin-left: auto;
    font-family: var(--font-num);
    font-size: 10px;
    color: var(--text-2);
  }
  kbd {
    padding: 0 5px;
    border-radius: 4px;
    border: 1px solid var(--line-strong);
    line-height: 16px;
  }
  .count {
    min-width: 18px;
    padding: 0 5px;
    border-radius: var(--radius-pill);
    background: var(--bg-4);
    color: var(--text-0);
    text-align: center;
    line-height: 18px;
  }
  .wb-mode {
    width: 100%;
  }
  .wb-mode button:disabled {
    cursor: default;
  }
  .spacer {
    height: var(--space-1);
  }
  .group {
    display: grid;
    gap: var(--space-2);
  }
  .group + .group {
    margin-top: var(--space-3);
    padding-top: var(--space-3);
    border-top: 1px solid var(--line);
  }
  .check {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--fs-s);
    color: var(--text-1);
    cursor: pointer;
  }
  .check input {
    margin: 0;
    accent-color: var(--text-1);
  }
  .note {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-2);
    line-height: 1.45;
  }
</style>
