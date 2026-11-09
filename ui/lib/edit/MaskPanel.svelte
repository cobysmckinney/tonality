<script lang="ts">
  // Masks: the list, and the selected mask's settings. Shown in place of the adjustments (M).
  import type { LocalAdjust } from "$lib/bindings/LocalAdjust";
  import type { Mask } from "$lib/bindings/Mask";
  import Slider from "$lib/design/Slider.svelte";
  import HistogramView from "./Histogram.svelte";
  import { KIND_LABEL, MAX_MASKS, type MaskKind } from "./masks";
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
  const groups: { title: string; controls: { key: AdjustKey; label: string; min: number; max: number; step?: number; decimals?: number; track?: string }[] }[] = [
    {
      title: "Light",
      controls: [
        { key: "exposure", label: "Exposure", min: -4, max: 4, step: 0.01, decimals: 2 },
        { key: "contrast", label: "Contrast", min: -100, max: 100 },
        { key: "highlights", label: "Highlights", min: -100, max: 100 },
        { key: "shadows", label: "Shadows", min: -100, max: 100 },
        { key: "whites", label: "Whites", min: -100, max: 100 },
        { key: "blacks", label: "Blacks", min: -100, max: 100 },
      ],
    },
    {
      title: "Color",
      controls: [
        { key: "temperature", label: "Temperature", min: -100, max: 100, track: "linear-gradient(90deg, #4f7fd9, #d8d8d8 50%, #e0a63c)" },
        { key: "tint", label: "Tint", min: -100, max: 100, track: "linear-gradient(90deg, #4fb55a, #d8d8d8 50%, #c45bc4)" },
        { key: "saturation", label: "Saturation", min: -100, max: 100 },
      ],
    },
  ];

  const adjusted = $derived(mask ? Object.values(mask.adjust).some((v) => v !== 0) : false);

  function resetAdjust() {
    if (!mask) return;
    for (const g of groups) for (const c of g.controls) mask.adjust[c.key] = 0;
    commit(null);
  }

  function toggle(i: number, field: "enabled" | "invert") {
    const m = session.edit?.masks?.[i];
    if (!m) return;
    m[field] = !m[field];
    commit(null);
  }

  // ---- rename ----
  let renaming = $state<number | null>(null);
  let draft = $state("");
  function startRename(i: number, m: Mask) {
    renaming = i;
    draft = m.name;
  }
  function finishRename() {
    const i = renaming;
    renaming = null;
    const m = i !== null ? session.edit?.masks?.[i] : undefined;
    const name = draft.trim();
    if (m && name && name !== m.name) {
      m.name = name;
      commit(null);
    }
  }
  function focus(el: HTMLInputElement) {
    el.focus();
    el.select();
  }

  const kinds: MaskKind[] = ["linear", "radial", "brush"];
  const icons: Record<MaskKind, string> = {
    linear: "M2 4h12M2 8h12M2 12h12",
    radial: "M8 2.5a5.5 4.5 0 1 0 0.01 0Z",
    brush: "M3 13c2 0 3-1 3-3l6-6.5a1.4 1.4 0 0 0-2-2L4 7.5c-2 0-3 1-3 3",
  };
  const hints: Record<MaskKind, string> = {
    linear: "Full effect beyond the solid line, fading to nothing at the faint one. Drag either end, or the pin to move it.",
    radial: "Drag inside to move, the edge handles to resize, the blue knob to rotate (Shift: 15° steps).",
    brush: "Paint over the image. Hold Alt to erase, Alt+wheel or [ ] to change size, Space to pan.",
  };
</script>

<div class="panel">
  <HistogramView {histogram} />

  <div class="scroll">
    <header>
      <span class="title">Masks</span>
      <button class="primary" onclick={() => session.stopMasks()} title="Back to the adjustments (M)">Done</button>
    </header>

    <div class="add">
      {#each kinds as k (k)}
        <button onclick={() => session.addMask(k)} disabled={masks.length >= MAX_MASKS} title="Add a {KIND_LABEL[k].toLowerCase()} mask">
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d={icons[k]} /></svg>
          {KIND_LABEL[k]}
        </button>
      {/each}
    </div>

    {#if masks.length}
      <ul class="list">
        {#each masks as m, i (i)}
          <li class:selected={sel === i} class:hidden={!m.enabled}>
            <button class="pick" onclick={() => (session.maskIndex = i)} ondblclick={() => startRename(i, m)}>
              <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d={icons[m.shape.kind]} /></svg>
              {#if renaming === i}
                <input
                  use:focus
                  bind:value={draft}
                  onblur={finishRename}
                  onkeydown={(e) => {
                    e.stopPropagation();
                    if (e.key === "Enter") finishRename();
                    if (e.key === "Escape") renaming = null;
                  }}
                />
              {:else}
                <span class="name">{m.name}</span>
              {/if}
            </button>
            <button class="icon" onclick={() => toggle(i, "enabled")} title={m.enabled ? "Hide" : "Show"}>
              {#if m.enabled}
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="M1 8s2.5-4.5 7-4.5S15 8 15 8s-2.5 4.5-7 4.5S1 8 1 8Z" /><circle cx="8" cy="8" r="2" /></svg>
              {:else}
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d="M2 2l12 12M6.3 4c.5-.2 1.1-.4 1.7-.4 4.5 0 7 4.4 7 4.4s-.7 1.2-2 2.4M4 5.3C2.2 6.5 1 8 1 8s2.5 4.5 7 4.5c1 0 2-.2 2.8-.6" /></svg>
              {/if}
            </button>
            <button class="icon" onclick={() => session.deleteMask(i)} title="Delete (Del)">×</button>
          </li>
        {/each}
      </ul>
    {:else}
      <p class="hint">Masks apply their own adjustments to part of the photo: a gradient across it, an area round a subject, or wherever you paint.</p>
    {/if}

    {#if mask && sel !== null}
      <div class="settings">
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
        {:else if mask.shape.kind === "brush"}
          <div class="modes">
            <button class:active={!session.brush.erase} onclick={() => (session.brush.erase = false)}>Paint</button>
            <button class:active={session.brush.erase} onclick={() => (session.brush.erase = true)}>Erase</button>
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

        <div class="checks">
          <label><input type="checkbox" checked={mask.invert} onchange={() => toggle(sel!, "invert")} /> Invert</label>
          <label title="O"><input type="checkbox" bind:checked={session.maskOverlay} /> Show overlay</label>
        </div>

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

        {#each groups as g (g.title)}
          <div class="sub">
            <span>{g.title}</span>
            {#if g.title === "Light" && adjusted}<button class="link" onclick={resetAdjust}>Reset</button>{/if}
          </div>
          {#each g.controls as c (c.key)}
            <Slider
              label={c.label}
              value={mask.adjust[c.key]}
              min={c.min}
              max={c.max}
              step={c.step ?? 1}
              decimals={c.decimals ?? 0}
              track={c.track}
              oninput={(v) => (mask.adjust[c.key] = v)}
              oncommit={() => commit(key(c.key))}
            />
          {/each}
        {/each}

        <div class="sub"><span>Luminance Range</span></div>
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
      </div>
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
  .scroll {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-3) var(--space-4) var(--space-5);
    display: grid;
    align-content: start;
    gap: var(--space-3);
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .title {
    font-weight: 500;
  }
  svg {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .add {
    display: grid;
    grid-template-columns: repeat(3, 1fr);
    gap: var(--space-1);
  }
  .add button {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding: 6px 0;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    color: var(--text-1);
    font-size: var(--fs-xs);
  }
  .add button:hover:not(:disabled) {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .add button:disabled {
    opacity: 0.4;
  }
  .list {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 1px;
  }
  li {
    display: flex;
    align-items: center;
    border-radius: var(--radius-s);
  }
  li:hover {
    background: var(--bg-2);
  }
  li.selected {
    background: var(--bg-3);
  }
  li.hidden .name,
  li.hidden .pick svg {
    opacity: 0.45;
  }
  .pick {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 5px var(--space-2);
    text-align: left;
    color: var(--text-0);
    font-size: var(--fs-s);
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .pick input {
    flex: 1;
    min-width: 0;
    font: inherit;
    color: inherit;
    background: var(--bg-0);
    border: 1px solid var(--line-strong);
    border-radius: var(--radius-s);
    padding: 0 4px;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 26px;
    height: 26px;
    color: var(--text-2);
  }
  .icon:hover {
    color: var(--text-0);
  }
  .settings {
    display: grid;
    gap: var(--space-2);
    padding-top: var(--space-2);
    border-top: 1px solid var(--line);
  }
  .sub {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    margin-top: var(--space-2);
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-2);
  }
  .checks {
    display: flex;
    gap: var(--space-4);
  }
  label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-1);
    cursor: pointer;
  }
  input[type="checkbox"] {
    accent-color: var(--text-1);
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
  .hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-2);
    line-height: 1.4;
  }
  .link {
    font-size: var(--fs-xs);
    color: var(--text-2);
    text-transform: none;
    letter-spacing: 0;
  }
  .link:hover {
    color: var(--text-0);
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
