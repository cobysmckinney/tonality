<!--
  Colour grading: one wheel with a dot each for the shadows, midtones and highlights.
  - Drag a dot: round the wheel for hue, out from the centre for strength. Hold Shift for fine
    control. Click anywhere else on the wheel to bring the selected dot there. Double-click a dot
    to send it back to the centre.
  - Link: hold the dots in a shape (see `grading.ts`), so dragging one turns and grows all three.
    Unlinking leaves them where they are.
-->
<script lang="ts">
  import type { ColorGrading } from "$lib/bindings/ColorGrading";
  import type { Harmony } from "$lib/bindings/Harmony";
  import Slider from "$lib/design/Slider.svelte";
  import {
    flipped,
    fromPoint,
    HARMONIES,
    heldAtCentre,
    linked,
    moved,
    RANGES,
    toPoint,
    type Range,
  } from "./grading";

  interface Props {
    value: ColorGrading;
    oninput: (g: ColorGrading) => void;
    /** Once per finished gesture; `key` names the control (`null` for one-off actions). */
    oncommit: (key: string | null) => void;
  }
  let { value, oninput, oncommit }: Props = $props();

  const labels: Record<Range, string> = {
    shadows: "Shadows",
    midtones: "Midtones",
    highlights: "Highlights",
  };
  let selected = $state<Range>("shadows");
  const range = $derived(value[selected]);
  const held = $derived(heldAtCentre(value, selected));

  // ---- the wheel ----
  // Sizes in px. The disc is inset so a dot on the rim isn't cut off.
  const SIZE = 188;
  const R = 82;
  const C = SIZE / 2;
  const DOT = 6.5;
  /** How near a press must be to a dot to pick it up, in wheel radii. */
  const REACH = 13 / R;

  // The wheel's colours are OkLCh hues, as the picture's tint is: hue 0 to the right, going
  // anticlockwise. (A conic gradient starts at the top and goes clockwise.)
  const TINT = "70% 0.13";
  const disc =
    `radial-gradient(circle closest-side, oklch(70% 0 0), oklch(70% 0 0 / 0)), ` +
    `conic-gradient(${Array.from({ length: 25 }, (_, i) => `oklch(${TINT} ${(450 - i * 15) % 360}) ${i * 15}deg`).join(", ")})`;

  const px = (r: Range) => {
    const p = toPoint(value[r]);
    return { x: C + p.x * R, y: C - p.y * R };
  };
  // The selected dot is drawn last, so it is on top where dots share a place.
  const order = $derived([...RANGES.filter((r) => r !== selected), selected]);
  // The shape joining the linked dots: neighbours in a row, the others all round.
  const outline = $derived.by(() => {
    if (!value.link || !RANGES.some((r) => value[r].saturation > 0)) return "";
    const on = RANGES.filter((r) => !heldAtCentre(value, r)).map(px);
    const d = on.map((p, i) => `${i ? "L" : "M"}${p.x.toFixed(1)},${p.y.toFixed(1)}`).join("");
    return value.link === "analogous" || on.length < 3 ? d : d + "Z";
  });

  let wheel: SVGSVGElement;
  // The dot being dragged and where the pointer has taken it (wheel radii, y up). Kept apart from
  // the dot itself so that Shift can slow the drag down.
  let drag: { range: Range; x: number; y: number; cx: number; cy: number; moved: boolean } | null =
    null;

  function at(e: PointerEvent | MouseEvent) {
    const b = wheel.getBoundingClientRect();
    return { x: (e.clientX - b.left - C) / R, y: -(e.clientY - b.top - C) / R };
  }
  /** The nearest dot that can be picked up (the selected one, where several share a place). */
  function nearest(p: { x: number; y: number }) {
    let best: { range: Range; d: number } | null = null;
    for (const r of [selected, ...RANGES]) {
      if (heldAtCentre(value, r)) continue;
      const q = toPoint(value[r]);
      const d = Math.hypot(q.x - p.x, q.y - p.y);
      if (!best || d < best.d - 1e-6) best = { range: r, d };
    }
    return best!;
  }
  function place(r: Range, x: number, y: number) {
    const { hue, saturation } = fromPoint(x, y);
    oninput(moved(value, r, hue, saturation));
  }

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    const p = at(e);
    const near = nearest(p);
    const grab = near.d <= REACH;
    // Away from every dot: the selected one comes to the pointer.
    const r = grab || heldAtCentre(value, selected) ? near.range : selected;
    selected = r;
    wheel.setPointerCapture(e.pointerId);
    const from = grab ? toPoint(value[r]) : p;
    drag = { range: r, x: from.x, y: from.y, cx: e.clientX, cy: e.clientY, moved: !grab };
    if (!grab) place(r, p.x, p.y);
  }
  function onPointerMove(e: PointerEvent) {
    if (!drag) return;
    const k = (e.shiftKey ? 0.2 : 1) / R;
    let x = drag.x + (e.clientX - drag.cx) * k;
    let y = drag.y - (e.clientY - drag.cy) * k;
    // Past the rim the dot waits there, so coming back in picks it up at once.
    const len = Math.hypot(x, y);
    if (len > 1) [x, y] = [x / len, y / len];
    if (x === drag.x && y === drag.y) return;
    drag = { ...drag, x, y, cx: e.clientX, cy: e.clientY, moved: true };
    place(drag.range, x, y);
  }
  function onPointerUp(e: PointerEvent) {
    if (!drag) return;
    wheel.releasePointerCapture(e.pointerId);
    if (drag.moved) oncommit("grading.wheel");
    drag = null;
  }
  function onDoubleClick(e: MouseEvent) {
    const near = nearest(at(e));
    if (near.d > REACH || value[near.range].saturation === 0) return;
    oninput(moved(value, near.range, value[near.range].hue, 0));
    oncommit(null);
  }

  // ---- link ----
  function link(h: Harmony) {
    oninput(linked(value, value.link === h ? null : h, selected));
    oncommit(null);
  }
  function flip() {
    oninput(flipped(value, selected));
    oncommit(null);
  }
  // Each shape as a small picture: its points round a circle, in degrees.
  const icons: Record<Harmony, { at: number[]; closed: boolean }> = {
    analogous: { at: [50, 90, 130], closed: false },
    complementary: { at: [30, 210], closed: false },
    triadic: { at: [90, 210, 330], closed: true },
    splitComplementary: { at: [270, 60, 120], closed: true },
  };
  const iconPoints = (h: Harmony) =>
    icons[h].at.map((a) => ({
      x: 11 + 7.5 * Math.cos((a * Math.PI) / 180),
      y: 11 - 7.5 * Math.sin((a * Math.PI) / 180),
    }));
  const iconPath = (h: Harmony) =>
    iconPoints(h)
      .map((p, i) => `${i ? "L" : "M"}${p.x.toFixed(2)},${p.y.toFixed(2)}`)
      .join("") + (icons[h].closed ? "Z" : "");

  // ---- sliders ----
  const hueTrack = `linear-gradient(90deg, ${Array.from({ length: 13 }, (_, i) => `oklch(${TINT} ${i * 30})`).join(", ")})`;
  const satTrack = $derived(`linear-gradient(90deg, oklch(70% 0 0), oklch(${TINT} ${range.hue}))`);
  function set(patch: Partial<ColorGrading>) {
    oninput({ ...value, ...patch });
  }
</script>

<div class="grading">
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div class="wheel" style:width="{SIZE}px" style:height="{SIZE}px">
    <div class="disc" style:inset="{C - R}px" style:background={disc}></div>
    <svg
      bind:this={wheel}
      width={SIZE}
      height={SIZE}
      viewBox="0 0 {SIZE} {SIZE}"
      role="img"
      aria-label="Colour wheel: shadows, midtones and highlights. The sliders below set the same values."
      onpointerdown={onPointerDown}
      onpointermove={onPointerMove}
      onpointerup={onPointerUp}
      onpointercancel={onPointerUp}
      ondblclick={onDoubleClick}
    >
      <circle class="centre" cx={C} cy={C} r="2" />
      {#if outline}
        <path class="shape under" d={outline} />
        <path class="shape" d={outline} />
      {/if}
      {#each order as r (r)}
        {@const p = px(r)}
        <g
          class="dot {r}"
          class:selected={r === selected}
          class:held={heldAtCentre(value, r)}
          transform="translate({p.x.toFixed(1)} {p.y.toFixed(1)})"
        >
          {#if r === selected}<circle class="halo" r={DOT + 3.5} />{/if}
          <circle class="body" r={heldAtCentre(value, r) ? DOT - 2.5 : DOT} />
        </g>
      {/each}
    </svg>
  </div>

  <div class="link">
    <div class="head">
      <span class="eyebrow">Link</span>
      <span class="state">
        {HARMONIES.find((h) => h.id === value.link)?.label ?? "Off"}
      </span>
      <button
        class="flip"
        disabled={!value.link || value.link === "complementary"}
        title="Flip the shape to its mirror image"
        onclick={flip}
      >
        Flip
      </button>
    </div>
    <div class="shapes">
      {#each HARMONIES as h (h.id)}
        <button
          class:active={value.link === h.id}
          aria-pressed={value.link === h.id}
          aria-label="Link: {h.label}"
          title={value.link === h.id ? `Unlink (${h.label})` : h.label}
          onclick={() => link(h.id)}
        >
          <svg viewBox="0 0 22 22" width="22" height="22" aria-hidden="true">
            <circle class="ring" cx="11" cy="11" r="7.5" />
            <path d={iconPath(h.id)} />
            {#each iconPoints(h.id) as p, i (i)}
              <circle class="pt" cx={p.x} cy={p.y} r="1.7" />
            {/each}
          </svg>
        </button>
      {/each}
    </div>
  </div>

  <div class="seg" role="tablist" aria-label="Range">
    {#each RANGES as r (r)}
      <button
        role="tab"
        aria-selected={selected === r}
        class:active={selected === r}
        onclick={() => (selected = r)}
      >
        <span class="swatch {r}"></span>{labels[r]}
      </button>
    {/each}
  </div>

  {#if held}
    <p class="note">On a line, the midtones stay at the centre: no tint.</p>
  {:else}
    <Slider
      label="Hue"
      value={range.hue}
      min={0}
      max={360}
      bipolar={false}
      unit="°"
      track={hueTrack}
      oninput={(v) => oninput(moved(value, selected, v, range.saturation))}
      oncommit={() => oncommit(`grading.${selected}.hue`)}
    />
    <Slider
      label="Saturation"
      value={range.saturation}
      min={0}
      max={100}
      bipolar={false}
      track={satTrack}
      oninput={(v) => oninput(moved(value, selected, range.hue, v))}
      oncommit={() => oncommit(`grading.${selected}.saturation`)}
    />
  {/if}
  <Slider
    label="Luminance"
    value={range.luminance}
    min={-100}
    max={100}
    oninput={(v) => set({ [selected]: { ...range, luminance: v } })}
    oncommit={() => oncommit(`grading.${selected}.luminance`)}
  />

  <div class="whole">
    <Slider
      label="Blending"
      value={value.blending}
      min={0}
      max={100}
      defaultValue={50}
      bipolar={false}
      oninput={(v) => set({ blending: v })}
      oncommit={() => oncommit("grading.blending")}
    />
    <Slider
      label="Balance"
      value={value.balance}
      min={-100}
      max={100}
      oninput={(v) => set({ balance: v })}
      oncommit={() => oncommit("grading.balance")}
    />
  </div>
</div>

<style>
  .grading {
    display: grid;
    /* Never wider than the panel, whatever a row would like to be. */
    grid-template-columns: minmax(0, 1fr);
    min-width: 0;
    gap: var(--space-2);
    /* The three dots, dark to light, as the tones they stand for. */
    --shadows: #2a2a2a;
    --midtones: #8a8a8a;
    --highlights: #f4f4f4;
  }
  .wheel {
    position: relative;
    justify-self: center;
  }
  .disc {
    position: absolute;
    border-radius: 50%;
    box-shadow:
      0 0 0 1px rgba(0, 0, 0, 0.5),
      0 0 0 2px var(--line-strong);
  }
  svg {
    position: relative;
    display: block;
    touch-action: none;
  }
  .wheel svg {
    cursor: crosshair;
  }
  .centre {
    fill: none;
    stroke: rgba(0, 0, 0, 0.35);
    stroke-width: 1;
  }
  .shape {
    fill: none;
    stroke: rgba(255, 255, 255, 0.9);
    stroke-width: 1;
    stroke-linejoin: round;
  }
  .shape.under {
    stroke: rgba(0, 0, 0, 0.45);
    stroke-width: 3;
  }
  .dot {
    cursor: grab;
  }
  .dot .body {
    stroke: rgba(255, 255, 255, 0.95);
    stroke-width: 1.5;
    filter: drop-shadow(0 0 1.5px rgba(0, 0, 0, 0.9));
  }
  .dot.shadows .body {
    fill: var(--shadows);
  }
  .dot.midtones .body {
    fill: var(--midtones);
  }
  .dot.highlights .body {
    fill: var(--highlights);
    stroke: rgba(0, 0, 0, 0.55);
  }
  .dot .halo {
    fill: none;
    stroke: rgba(255, 255, 255, 0.9);
    stroke-width: 1;
    filter: drop-shadow(0 0 1px rgba(0, 0, 0, 0.9));
  }
  .dot.held {
    cursor: default;
    opacity: 0.8;
  }

  .link {
    display: grid;
    gap: var(--space-1);
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    min-height: 20px;
  }
  .shapes {
    display: flex;
    gap: 2px;
    padding: 3px;
    border-radius: var(--radius-m);
    background: var(--bg-0);
  }
  .shapes button {
    flex: 1;
    display: grid;
    place-items: center;
    height: 26px;
    border-radius: var(--radius-s);
    color: var(--text-2);
    transition:
      background 120ms var(--ease),
      color 120ms var(--ease);
  }
  .shapes button:hover {
    color: var(--text-0);
  }
  .shapes button.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .shapes svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.1;
    stroke-linejoin: round;
  }
  .shapes .ring {
    opacity: 0.35;
  }
  .shapes .pt {
    fill: currentColor;
    stroke: none;
  }
  .state {
    flex: 1;
    min-width: 0;
    font-size: var(--fs-xs);
    color: var(--text-1);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .flip {
    padding: 2px 8px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--text-1);
  }
  .flip:hover:not(:disabled) {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .flip:disabled {
    opacity: 0.35;
  }

  .seg button {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
    padding-left: 4px;
    padding-right: 4px;
  }
  .swatch {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
    box-shadow: 0 0 0 1px rgba(255, 255, 255, 0.35);
  }
  .swatch.shadows {
    background: var(--shadows);
  }
  .swatch.midtones {
    background: var(--midtones);
  }
  .swatch.highlights {
    background: var(--highlights);
  }
  .note {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-2);
    line-height: 1.45;
  }
  .whole {
    display: grid;
    gap: var(--space-2);
    margin-top: var(--space-1);
    padding-top: var(--space-3);
    border-top: 1px solid var(--line);
  }
</style>
