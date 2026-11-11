<script lang="ts">
  // Masks: the list, and the selected mask's settings. Shown in place of the adjustments (M).
  import type { LocalAdjust } from "$lib/bindings/LocalAdjust";
  import type { Mask } from "$lib/bindings/Mask";
  import type { MaskShape } from "$lib/bindings/MaskShape";
  import Slider from "$lib/design/Slider.svelte";
  import HistogramView from "./Histogram.svelte";
  import { AI_LABEL, KIND_LABEL, MAX_MASKS, aiKey, type AiPreset, type MaskKind } from "./masks";
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
  // Renames are by position: if the list changes under one (a delete, undo), drop it rather
  // than rename whichever mask moved into that slot.
  $effect(() => {
    void masks.length;
    renaming = null;
  });
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
  const detects: { id: AiPreset; title: string }[] = [
    { id: "subject", title: "The photo's main subject" },
    { id: "background", title: "Everything but the main subject" },
    { id: "sky", title: "The sky" },
    { id: "people", title: "Every person in the photo" },
    { id: "object", title: "Circle something to select it" },
  ];
  const aiIcons: Record<AiPreset, string> = {
    subject: "M8 2.5a2.6 2.6 0 1 0 .01 0ZM3 14c0-3 2.2-5 5-5s5 2 5 5",
    background: "M2 2h12v12H2ZM8 5a2 2 0 1 0 .01 0ZM4.5 14c.4-2.2 1.8-3.5 3.5-3.5s3.1 1.3 3.5 3.5",
    sky: "M4 11a3 3 0 0 1 .4-6A4 4 0 0 1 12 6a2.5 2.5 0 0 1 0 5ZM2 14h12",
    people: "M5.5 3a2 2 0 1 0 .01 0ZM11 4a1.7 1.7 0 1 0 .01 0ZM1.5 13c0-2.5 1.8-4 4-4s4 1.5 4 4M9.5 9.3c.5-.2 1-.3 1.5-.3 2 0 3.5 1.3 3.5 3.5",
    object: "M8 2c3.6 0 6 2.4 6 5.5S11.4 13 8 13s-6-2.5-6-5.5c0-1 .3-2 .9-2.8M8 6v3M6.5 7.5h3",
  };
  const pick = (p: AiPreset) => (p === "object" ? session.startLasso() : session.addAiMask(p));

  function status(m: Mask) {
    const k = aiKey(m);
    return k ? session.aiStatus.get(k) : undefined;
  }
  const progress = $derived(session.aiProgress);
  const pct = (p: { done: number; total: number }) => Math.round((100 * p.done) / Math.max(p.total, 1));
  const icons: Record<MaskShape["kind"], string> = {
    ai: "M8 1.5l1.6 4.9 4.9 1.6-4.9 1.6L8 14.5l-1.6-4.9L1.5 8l4.9-1.6Z",
    linear: "M2 4h12M2 8h12M2 12h12",
    radial: "M8 2.5a5.5 4.5 0 1 0 0.01 0Z",
    brush: "M3 13c2 0 3-1 3-3l6-6.5a1.4 1.4 0 0 0-2-2L4 7.5c-2 0-3 1-3 3",
  };
  const hints: Record<MaskShape["kind"], string> = {
    ai: "Found automatically. Paint to add to it, hold Alt to erase.",
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

    <div class="detect">
      <span class="eyebrow">Detect</span>
      <div class="detect-grid">
        {#each detects as d (d.id)}
          <button
            class:on={d.id === "object" && session.lassoing !== null}
            onclick={() => pick(d.id)}
            disabled={masks.length >= MAX_MASKS}
            title={d.title}
          >
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d={aiIcons[d.id]} /></svg>
            {d.id === "object" ? "Select Object" : AI_LABEL[d.id]}
          </button>
        {/each}
      </div>
    </div>

    {#if masks.length}
      <ul class="list">
        {#each masks as m, i (i)}
          <li class:selected={sel === i} class:hidden={!m.enabled}>
            {#if renaming === i}
              <!-- Not inside the button: an <input> in a <button> is invalid and edits unreliably. -->
              <div class="pick">
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d={icons[m.shape.kind]} /></svg>
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
              </div>
            {:else}
              {@const st = status(m)}
              <button class="pick" onclick={() => (session.maskIndex = i)} ondblclick={() => startRename(i, m)}>
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"><path d={icons[m.shape.kind]} /></svg>
                <span class="name">{m.name}</span>
                {#if st?.state === "working"}
                  <span class="ai-state working" title="Detecting…">
                    {#if progress?.stage === "download"}{pct(progress)}%{:else}<span class="spinner"></span>{/if}
                  </span>
                {:else if st?.state === "empty"}
                  <span class="ai-state warn" title="The model didn't find anything">Not found</span>
                {:else if st?.state === "error"}
                  <span class="ai-state warn" title={st.message}>Failed</span>
                {/if}
              </button>
            {/if}
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
        {:else if mask.shape.kind === "ai"}
          {@const st = status(mask)}
          {#if st?.state === "working"}
            <p class="ai-note">
              {#if progress?.stage === "download"}
                Downloading the model… {pct(progress)}%
                <span class="bar"><span style:width="{pct(progress)}%"></span></span>
              {:else}
                Finding {mask.shape.target.kind === "object" ? "the object" : AI_LABEL[mask.shape.target.kind].toLowerCase()}…
              {/if}
            </p>
          {:else if st?.state === "empty"}
            <p class="ai-note warn">
              No {mask.shape.target.kind === "people" ? "people" : mask.shape.target.kind} found in this photo. Paint to add areas by hand.
            </p>
          {:else if st?.state === "error"}
            <p class="ai-note warn">Detection failed: {st.message}</p>
            <button class="ghost" onclick={() => session.detect(mask)}>Try again</button>
          {/if}
          {#if mask.shape.target.kind === "object"}
            <button class="ghost" onclick={() => session.startLasso(sel)}>Circle again…</button>
          {/if}
          <div class="modes">
            <button class:active={!session.brush.erase} onclick={() => (session.brush.erase = false)}>Add</button>
            <button class:active={session.brush.erase} onclick={() => (session.brush.erase = true)}>Erase</button>
          </div>
          <Slider
            label="Brush Size"
            value={Math.round(session.brush.size * 1000) / 10}
            min={0.3}
            max={30}
            step={0.1}
            decimals={1}
            defaultValue={4}
            bipolar={false}
            oninput={(v) => (session.brush.size = v / 100)}
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
            {#if g === groups[0] && adjusted}<button class="link" onclick={resetAdjust} title="Reset this mask's Light and Color">Reset all</button>{/if}
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
  .detect {
    display: grid;
    gap: var(--space-2);
  }
  .detect-grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-1);
  }
  .detect-grid button {
    display: flex;
    align-items: center;
    gap: 8px;
    height: 32px;
    padding: 0 10px;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    color: var(--text-1);
    font-size: var(--fs-s);
  }
  .detect-grid button:last-child {
    grid-column: span 2;
  }
  .detect-grid button:hover:not(:disabled),
  .detect-grid button.on {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .detect-grid button:disabled {
    opacity: 0.4;
  }
  .ai-state {
    margin-left: auto;
    font-size: var(--fs-xs);
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .ai-state.warn,
  .ai-note.warn {
    color: #e0a060;
  }
  .spinner {
    display: inline-block;
    width: 10px;
    height: 10px;
    border-radius: 50%;
    border: 1.5px solid var(--text-2);
    border-top-color: transparent;
    animation: spin 0.8s linear infinite;
    vertical-align: middle;
  }
  @keyframes spin {
    to {
      transform: rotate(360deg);
    }
  }
  .ai-note {
    display: grid;
    gap: 6px;
    margin: 0;
    font-size: var(--fs-xs);
    line-height: 1.45;
    color: var(--text-1);
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
