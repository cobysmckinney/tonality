<script lang="ts">
  // Mask mode's side panel: the selected mask's settings (the masks themselves float over the
  // photo, in MaskList). With no masks yet, the ways to make one.
  import type { LocalAdjust } from "$lib/bindings/LocalAdjust";
  import type { Mask } from "$lib/bindings/Mask";
  import Section from "$lib/design/Section.svelte";
  import Slider from "$lib/design/Slider.svelte";
  import HistogramView from "./Histogram.svelte";
  import MaskCreate from "./MaskCreate.svelte";
  import { AI_LABEL, KIND_LABEL, SHAPE_ICON, aiKey } from "./masks";
  import type { Histogram } from "./renderer";
  import { session } from "./session.svelte";

  interface Props {
    histogram: Histogram | null;
  }
  let { histogram }: Props = $props();

  const masks = $derived(session.masks);
  const sel = $derived(session.selected);
  const mask = $derived(sel !== null ? session.edit?.masks?.[sel] : undefined);

  const commit = (key: string | null) => session.commit(key);
  const key = (field: string) => `mask${sel}.${field}`;

  type AdjustKey = keyof LocalAdjust;
  type Control = {
    key: AdjustKey;
    label: string;
    min: number;
    max: number;
    step?: number;
    decimals?: number;
    track?: string;
  };
  const light: Control[] = [
    { key: "exposure", label: "Exposure", min: -4, max: 4, step: 0.01, decimals: 2 },
    { key: "contrast", label: "Contrast", min: -100, max: 100 },
    { key: "highlights", label: "Highlights", min: -100, max: 100 },
    { key: "shadows", label: "Shadows", min: -100, max: 100 },
    { key: "whites", label: "Whites", min: -100, max: 100 },
    { key: "blacks", label: "Blacks", min: -100, max: 100 },
  ];
  const color: Control[] = [
    {
      key: "temperature",
      label: "Temperature",
      min: -100,
      max: 100,
      track: "linear-gradient(90deg, #4f7fd9, #d8d8d8 50%, #e0a63c)",
    },
    {
      key: "tint",
      label: "Tint",
      min: -100,
      max: 100,
      track: "linear-gradient(90deg, #4fb55a, #d8d8d8 50%, #c45bc4)",
    },
    {
      key: "hue",
      label: "Hue",
      min: -100,
      max: 100,
      track: "linear-gradient(90deg, #c46bd0, #d8d8d8 50%, #d8b04f)",
    },
    { key: "saturation", label: "Saturation", min: -100, max: 100 },
    { key: "vibrance", label: "Vibrance", min: -100, max: 100 },
  ];
  const detail: Control[] = [
    { key: "sharpness", label: "Sharpness", min: -100, max: 100 },
    { key: "noise", label: "Noise Reduction", min: -100, max: 100 },
  ];
  const effects: Control[] = [
    { key: "texture", label: "Texture", min: -100, max: 100 },
    { key: "clarity", label: "Clarity", min: -100, max: 100 },
    { key: "dehaze", label: "Dehaze", min: -100, max: 100 },
  ];
  const tintKeys: AdjustKey[] = ["colorHue", "colorAmount"];
  // Texture, Clarity, Dehaze are left out at 0 (as Rust saves them, so older masks keep their
  // hashes); the others are always there.
  const OPTIONAL = new Set<AdjustKey>(["texture", "clarity", "dehaze"]);
  const get = (k: AdjustKey) => mask?.adjust[k] ?? 0;
  function set(k: AdjustKey, v: number) {
    if (!mask) return;
    if (v === 0 && OPTIONAL.has(k)) delete mask.adjust[k];
    else mask.adjust[k] = v;
  }
  // The tint's hue is OkLCh, like the colour mixer: the track shows those hues.
  const hueTrack = `linear-gradient(90deg, ${Array.from({ length: 13 }, (_, i) => `oklch(72% 0.14 ${i * 30})`).join(", ")})`;
  const tintSwatch = (h: number) => `oklch(72% 0.14 ${h})`;

  const edited = $derived({
    mask: mask ? mask.amount !== 100 || mask.invert : false,
    light: mask ? light.some((c) => mask.adjust[c.key] !== 0) : false,
    color: mask ? color.some((c) => mask.adjust[c.key] !== 0) : false,
    tint: mask ? mask.adjust.colorAmount !== 0 : false,
    detail: mask ? detail.some((c) => mask.adjust[c.key] !== 0) : false,
    effects: mask ? effects.some((c) => get(c.key) !== 0) : false,
    range: mask
      ? mask.range.low !== 0 || mask.range.high !== 100 || mask.range.smoothness !== 20
      : false,
  });

  function zero(keys: AdjustKey[]) {
    if (!mask) return;
    for (const k of keys) set(k, 0);
    commit(null);
  }
  const keysOf = (cs: Control[]) => cs.map((c) => c.key);
  function resetMask() {
    if (!mask) return;
    mask.amount = 100;
    mask.invert = false;
    commit(null);
  }
  function resetRange() {
    if (!mask) return;
    mask.range = { low: 0, high: 100, smoothness: 20 };
    commit(null);
  }
  function toggleInvert() {
    if (!mask) return;
    mask.invert = !mask.invert;
    commit(null);
  }

  function status(m: Mask) {
    const k = aiKey(m);
    return k ? session.aiStatus.get(k) : undefined;
  }
  const progress = $derived(session.aiProgress);
  const pct = (p: { done: number; total: number }) =>
    Math.round((100 * p.done) / Math.max(p.total, 1));

  const kindName = (m: Mask) =>
    m.shape.kind === "ai"
      ? m.shape.target.kind === "object"
        ? "Object (AI)"
        : `${AI_LABEL[m.shape.target.kind]} (AI)`
      : m.shape.kind === "brush"
        ? "Brush"
        : `${KIND_LABEL[m.shape.kind]} Gradient`;

  const shapeTitle: Record<Mask["shape"]["kind"], string> = {
    ai: "Refine",
    linear: "Gradient",
    radial: "Gradient",
    brush: "Brush",
  };
  const hints: Record<Mask["shape"]["kind"], string> = {
    ai: "Found automatically. Paint to add to it, hold Alt to erase.",
    linear:
      "Full effect beyond the solid line, fading to nothing at the faint one. Drag either end, or the pin to move it.",
    radial:
      "Drag inside to move, the edge handles to resize, the blue knob to rotate (Shift: 15° steps).",
    brush:
      "Paint over the image. Hold Alt to erase, Alt+wheel or [ ] to change size, Space to pan.",
  };
</script>

<div class="panel">
  <div class="top">
    <HistogramView {histogram} />
    <header>
      {#if mask}
        <span class="kind-icon">
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
            <path d={SHAPE_ICON[mask.shape.kind]} />
          </svg>
        </span>
        <span class="heading">
          <span class="name">{mask.name}</span>
          <span class="sub">{kindName(mask)}</span>
        </span>
      {:else}
        <span class="heading"><span class="name">Masks</span></span>
      {/if}
      <button
        class="primary"
        onclick={() => session.stopMasks()}
        title="Back to the adjustments (M)"
      >
        Done
      </button>
    </header>
  </div>

  <div class="scroll">
    {#if !masks.length}
      <div class="empty">
        <p class="hint">
          Masks apply their own adjustments to part of the photo: a gradient across it, an area
          round a subject, or wherever you paint.
        </p>
        <MaskCreate />
      </div>
    {:else if !mask || sel === null}
      <p class="hint pad">Pick a mask in the list over the photo to adjust it.</p>
    {:else}
      <Section title={shapeTitle[mask.shape.kind]} storeKey="mask.shape">
        <p class="hint">{hints[mask.shape.kind]}</p>
        {#if mask.shape.kind === "radial"}
          {@const shape = mask.shape}
          <Slider
            label="Feather"
            value={Math.round(shape.feather * 100)}
            min={0}
            max={100}
            defaultValue={50}
            bipolar={false}
            oninput={(v) => (shape.feather = v / 100)}
            oncommit={() => commit(key("feather"))}
          />
        {:else if mask.shape.kind === "ai" || mask.shape.kind === "brush"}
          {#if mask.shape.kind === "ai"}
            {@const st = status(mask)}
            {@const target = mask.shape.target.kind}
            {#if st?.state === "working"}
              <p class="ai-note">
                {#if progress?.stage === "download"}
                  Downloading the model… {pct(progress)}%
                  <span class="bar"><span style:width="{pct(progress)}%"></span></span>
                {:else}
                  Finding {target === "object" ? "the object" : AI_LABEL[target].toLowerCase()}…
                {/if}
              </p>
            {:else if st?.state === "empty"}
              <p class="ai-note warn">
                No {target === "people" ? "people" : target} found in this photo. Paint to add areas by
                hand.
              </p>
            {:else if st?.state === "error"}
              <p class="ai-note warn">Detection failed.</p>
              <button class="ghost" onclick={() => session.detect(mask)}>Try again</button>
            {:else if st?.state === "declined"}
              <p class="ai-note warn">
                This mask does nothing yet: it needs a model that isn’t downloaded.
              </p>
              <button class="ghost" onclick={() => session.downloadAndDetect(mask)}>
                Download…
              </button>
            {/if}
            {#if target === "object"}
              <button class="ghost" onclick={() => session.startLasso(sel)}>Circle again…</button>
            {/if}
          {/if}
          <div class="modes">
            <button
              class:active={!session.brush.erase}
              aria-pressed={!session.brush.erase}
              onclick={() => (session.brush.erase = false)}
            >
              {mask.shape.kind === "ai" ? "Add" : "Paint"}
            </button>
            <button
              class:active={session.brush.erase}
              aria-pressed={session.brush.erase}
              onclick={() => (session.brush.erase = true)}
            >
              Erase
            </button>
          </div>
          <Slider
            label="Size"
            value={Math.round(session.brush.size * 1000) / 10}
            min={0.3}
            max={30}
            step={0.1}
            decimals={1}
            defaultValue={4}
            bipolar={false}
            oninput={(v) => (session.brush.size = v / 100)}
          />
          {#if mask.shape.kind === "brush"}
            <Slider
              label="Feather"
              value={Math.round(session.brush.feather * 100)}
              min={0}
              max={100}
              defaultValue={60}
              bipolar={false}
              oninput={(v) => (session.brush.feather = v / 100)}
            />
            <Slider
              label="Strength"
              value={Math.round(session.brush.strength * 100)}
              min={1}
              max={100}
              defaultValue={100}
              bipolar={false}
              oninput={(v) => (session.brush.strength = v / 100)}
            />
          {/if}
        {/if}
      </Section>

      <Section title="Mask" storeKey="mask.mask" edited={edited.mask} onreset={resetMask}>
        <Slider
          label="Amount"
          value={mask.amount}
          min={0}
          max={100}
          defaultValue={100}
          bipolar={false}
          oninput={(v) => (mask.amount = v)}
          oncommit={() => commit(key("amount"))}
        />
        <button
          class="toggle"
          class:active={mask.invert}
          onclick={toggleInvert}
          aria-pressed={mask.invert}
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <circle cx="8" cy="8" r="6" /><path d="M8 2a6 6 0 0 0 0 12Z" fill="currentColor" />
          </svg>
          Invert
        </button>
      </Section>

      <Section
        title="Light"
        storeKey="mask.light"
        edited={edited.light}
        onreset={() => zero(keysOf(light))}
      >
        {#each light as c (c.key)}
          <Slider
            label={c.label}
            value={get(c.key)}
            min={c.min}
            max={c.max}
            step={c.step ?? 1}
            decimals={c.decimals ?? 0}
            oninput={(v) => set(c.key, v)}
            oncommit={() => commit(key(c.key))}
          />
        {/each}
      </Section>

      <Section
        title="Color"
        storeKey="mask.color"
        edited={edited.color}
        onreset={() => zero(keysOf(color))}
      >
        {#each color as c (c.key)}
          <Slider
            label={c.label}
            value={get(c.key)}
            min={c.min}
            max={c.max}
            track={c.track}
            oninput={(v) => set(c.key, v)}
            oncommit={() => commit(key(c.key))}
          />
        {/each}
      </Section>

      <Section
        title="Color Tint"
        storeKey="mask.tint"
        open={false}
        edited={edited.tint}
        onreset={() => zero(tintKeys)}
      >
        <p class="hint">Wash the area with a colour, keeping its brightness.</p>
        <div class="tint-row">
          <span
            class="swatch"
            style:background={tintSwatch(mask.adjust.colorHue)}
            style:opacity={0.35 + 0.65 * (mask.adjust.colorAmount / 100)}
          ></span>
          <div class="grow">
            <Slider
              label="Hue"
              value={mask.adjust.colorHue}
              min={0}
              max={360}
              defaultValue={0}
              bipolar={false}
              track={hueTrack}
              oninput={(v) => (mask.adjust.colorHue = v)}
              oncommit={() => commit(key("colorHue"))}
            />
          </div>
        </div>
        <Slider
          label="Amount"
          value={mask.adjust.colorAmount}
          min={0}
          max={100}
          defaultValue={0}
          bipolar={false}
          oninput={(v) => (mask.adjust.colorAmount = v)}
          oncommit={() => commit(key("colorAmount"))}
        />
      </Section>

      <Section
        title="Detail"
        storeKey="mask.detail"
        open={false}
        edited={edited.detail}
        onreset={() => zero(keysOf(detail))}
      >
        {#each detail as c (c.key)}
          <Slider
            label={c.label}
            value={get(c.key)}
            min={c.min}
            max={c.max}
            oninput={(v) => set(c.key, v)}
            oncommit={() => commit(key(c.key))}
          />
        {/each}
      </Section>

      <Section
        title="Effects"
        storeKey="mask.effects"
        open={false}
        edited={edited.effects}
        onreset={() => zero(keysOf(effects))}
      >
        {#each effects as c (c.key)}
          <Slider
            label={c.label}
            value={get(c.key)}
            min={c.min}
            max={c.max}
            oninput={(v) => set(c.key, v)}
            oncommit={() => commit(key(c.key))}
          />
        {/each}
      </Section>

      <Section
        title="Luminance Range"
        storeKey="mask.range"
        open={false}
        edited={edited.range}
        onreset={resetRange}
      >
        <p class="hint">Limit the mask to tones between these levels.</p>
        <Slider
          label="Darkest"
          value={mask.range.low}
          min={0}
          max={100}
          bipolar={false}
          oninput={(v) => (mask.range.low = Math.min(v, mask.range.high))}
          oncommit={() => commit(key("range"))}
        />
        <Slider
          label="Brightest"
          value={mask.range.high}
          min={0}
          max={100}
          defaultValue={100}
          bipolar={false}
          oninput={(v) => (mask.range.high = Math.max(v, mask.range.low))}
          oncommit={() => commit(key("range"))}
        />
        <Slider
          label="Smoothness"
          value={mask.range.smoothness}
          min={0}
          max={100}
          defaultValue={20}
          bipolar={false}
          oninput={(v) => (mask.range.smoothness = v)}
          oncommit={() => commit(key("range"))}
        />
      </Section>
      <div class="end"></div>
    {/if}
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
  header {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-4);
  }
  .kind-icon {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    color: var(--text-1);
  }
  .heading {
    flex: 1;
    min-width: 0;
    display: grid;
  }
  .name {
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .sub {
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding-top: var(--space-2);
  }
  .empty {
    display: grid;
    gap: var(--space-3);
    padding: var(--space-2) var(--space-3);
  }
  .empty .hint {
    padding: 0 var(--space-2);
  }
  .pad {
    padding: var(--space-2) var(--space-4);
  }
  .end {
    height: var(--space-5);
  }
  svg {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .ai-note {
    display: grid;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-xs);
    line-height: 1.45;
    color: var(--text-1);
  }
  .ai-note.warn {
    color: #e0a060;
  }
  .bar {
    display: block;
    height: 3px;
    border-radius: 2px;
    background: var(--bg-3);
    overflow: hidden;
  }
  .bar span {
    display: block;
    height: 100%;
    background: var(--text-1);
    transition: width 120ms linear;
  }
  .ghost {
    justify-self: start;
    padding: 4px 10px;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    color: var(--text-1);
    font-size: var(--fs-xs);
  }
  .ghost:hover {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .modes {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--bg-0);
  }
  .modes button {
    padding: 4px 0;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .modes button.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .toggle {
    justify-self: start;
    display: flex;
    align-items: center;
    gap: 6px;
    padding: 4px 10px;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    color: var(--text-1);
    font-size: var(--fs-s);
  }
  .toggle:hover {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .toggle.active {
    background: var(--text-1);
    color: var(--bg-0);
  }
  .tint-row {
    display: flex;
    align-items: flex-end;
    gap: var(--space-3);
  }
  .grow {
    flex: 1;
    min-width: 0;
  }
  .swatch {
    flex: none;
    width: 22px;
    height: 22px;
    margin-bottom: 4px;
    border-radius: 50%;
    box-shadow: inset 0 0 0 1px rgb(255 255 255 / 0.15);
  }
  .hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-2);
    line-height: 1.4;
  }
  .primary {
    padding: 4px 14px;
    border-radius: var(--radius-m);
    background: var(--bg-3);
    color: var(--text-0);
    font-weight: 500;
  }
  .primary:hover {
    background: var(--line-strong);
  }
</style>
