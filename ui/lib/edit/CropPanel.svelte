<script lang="ts">
  // Crop & rotate controls, shown in place of the adjustments while cropping.
  import type { Crop } from "$lib/bindings/Crop";
  import type { ImageInfo } from "$lib/bindings/ImageInfo";
  import Slider from "$lib/design/Slider.svelte";
  import {
    ASPECTS,
    FULL,
    flip,
    shrinkToFit,
    turn,
    turnedSize,
    withAspect,
    type Rect,
  } from "./crop";
  import { session } from "./session.svelte";
  import { untrack } from "svelte";

  interface Props {
    image: ImageInfo;
    level: boolean;
    ontogglelevel: () => void;
  }
  let { image, level, ontogglelevel }: Props = $props();

  const crop = $derived(session.edit!.crop);
  const turned = $derived(turnedSize(crop, image.fullWidth, image.fullHeight));
  const size = $derived.by(() => {
    const [tw, th] = turned;
    return `${Math.round(crop.rect[2] * tw)} × ${Math.round(crop.rect[3] * th)}`;
  });

  function set(c: Crop) {
    session.edit!.crop = c;
  }

  // ---- aspect ----
  let preset = $state<string>("Free");
  // Starts as the crop box is, so the icons and labels match what's on screen.
  let portrait = $state(
    untrack(() => {
      const [tw, th] = turnedSize(session.edit!.crop, image.fullWidth, image.fullHeight);
      const r = session.edit!.crop.rect;
      return r[2] * tw < r[3] * th;
    }),
  );
  // Rotating with [ ] (in Edit) turns the locked aspect: follow it.
  $effect(() => {
    const a = session.cropAspect;
    if (a !== null && a !== 1) portrait = a < 1;
  });

  /** Presets with no orientation of their own. */
  const square = (label: string) => label === "Free" || label === "1:1";

  /** "4:5" shown the way round it will crop: "5:4" in landscape. */
  function labelFor(label: string, port: boolean): string {
    const m = label.match(/^(\d+):(\d+)$/);
    if (!m) return label;
    const [a, b] = [Number(m[1]), Number(m[2])];
    const [lo, hi] = [Math.min(a, b), Math.max(a, b)];
    return port ? `${lo}:${hi}` : `${hi}:${lo}`;
  }

  /** The icon's rectangle (in a 28 × 28 box) for a preset, the way round it will crop. */
  function iconRect(label: string, port: boolean) {
    const r = label === "Free" ? 1.4 : (ratioFor(label, port) ?? 1);
    const m = 20;
    const [w, h] = r >= 1 ? [m, m / r] : [m * r, m];
    return { x: 14 - w / 2, y: 14 - h / 2, w, h };
  }

  function ratioFor(label: string, port: boolean): number | null {
    const a = ASPECTS.find((x) => x.label === label)!;
    if (a.ratio === null) return null;
    const r = a.ratio === "original" ? image.fullWidth / image.fullHeight : a.ratio;
    // A preset names a shape; the orientation is separate (⇄).
    const land = Math.max(r, 1 / r);
    return port ? 1 / land : land;
  }

  function choose(label: string) {
    preset = label;
    const a = ASPECTS.find((x) => x.label === label)!;
    if (a.ratio !== null) {
      // Keep the box's current orientation.
      const [tw, th] = turned;
      portrait = crop.rect[2] * tw < crop.rect[3] * th;
    }
    apply();
  }

  function apply() {
    const ratio = ratioFor(preset, portrait);
    session.cropAspect = ratio;
    if (ratio !== null)
      set({ ...crop, rect: withAspect(crop.rect as Rect, ratio, crop.angle, ...turned) });
  }

  function swap() {
    portrait = !portrait;
    apply();
  }

  // ---- straighten ----
  // The crop is shrunk to fit each new angle from `base`, the rectangle when the drag began.
  let base: Rect | null = null;
  // The last drag's `base`, and when that drag ended. A double-click to reset is two presses
  // first: the first one jumps the slider to the pointer and shrinks the crop for that angle.
  // The reset goes back to the rectangle from before those presses, not the shrunken one.
  let ended: { base: Rect; at: number } | null = null;
  const DOUBLE_CLICK_MS = 600;
  function straighten(angle: number) {
    base ??= [...(crop.rect as Rect)] as Rect;
    set({ ...crop, angle, rect: shrinkToFit(base, angle, ...turned) });
  }
  function endStraighten() {
    if (base) ended = { base, at: performance.now() };
    base = null;
  }
  function resetStraighten() {
    const before = ended && performance.now() - ended.at < DOUBLE_CLICK_MS ? ended.base : null;
    base = before;
    straighten(0);
    base = null;
    ended = null;
  }

  function rotate(dir: 1 | -1) {
    set(turn(crop, dir));
    if (session.cropAspect !== null) {
      portrait = !portrait;
      session.cropAspect = 1 / session.cropAspect;
    }
  }

  function reset() {
    set({ turns: 0, flip: false, angle: 0, rect: [...FULL] });
    preset = "Free";
    session.cropAspect = null;
  }
</script>

<div class="panel">
  <header>
    <span class="title">Crop &amp; Rotate</span>
    <span class="size">{size}</span>
  </header>

  <div class="group">
    <div class="aspect-head">
      <span class="sub">Aspect</span>
      <div class="orient" role="radiogroup" aria-label="Orientation">
        <button
          role="radio"
          aria-checked={!portrait}
          class:active={!portrait}
          onclick={() => portrait && swap()}
          disabled={square(preset)}
          title="Landscape"
        >
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
            <rect x="2" y="4.5" width="12" height="7" rx="1.2" />
          </svg>
        </button>
        <button
          role="radio"
          aria-checked={portrait}
          class:active={portrait}
          onclick={() => !portrait && swap()}
          disabled={square(preset)}
          title="Portrait"
        >
          <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true">
            <rect x="4.5" y="2" width="7" height="12" rx="1.2" />
          </svg>
        </button>
      </div>
    </div>
    <div class="aspects" role="radiogroup" aria-label="Aspect ratio">
      {#each ASPECTS as a (a.label)}
        {@const r = iconRect(a.label, portrait)}
        <button
          role="radio"
          aria-checked={preset === a.label}
          class:active={preset === a.label}
          onclick={() => choose(a.label)}
          title={a.label === "Free"
            ? "Any shape"
            : a.label === "Original"
              ? "The photo's own shape"
              : `${labelFor(a.label, portrait)}`}
        >
          <svg viewBox="0 0 28 28" width="28" height="28" aria-hidden="true">
            <rect
              x={r.x}
              y={r.y}
              width={r.w}
              height={r.h}
              rx="1.5"
              class:dashed={a.label === "Free"}
            />
            {#if a.label === "Original"}
              <!-- A little landscape: the photo as shot. -->
              <path
                d="M{r.x + 2.5} {r.y + r.h - 2.5}l{r.w * 0.28} {-r.h * 0.38}l{r.w * 0.18} {r.h *
                  0.2}l{r.w * 0.14} {-r.h * 0.14}l{r.w * 0.22} {r.h * 0.32}"
              />
            {/if}
          </svg>
          <span>{labelFor(a.label, portrait)}</span>
        </button>
      {/each}
    </div>
  </div>

  <div class="group">
    <Slider
      label="Straighten"
      value={crop.angle}
      min={-45}
      max={45}
      step={0.1}
      decimals={1}
      unit="°"
      oninput={straighten}
      oncommit={endStraighten}
      onreset={resetStraighten}
    />
    <button
      class="tool"
      class:active={level}
      aria-pressed={level}
      onclick={ontogglelevel}
      title="Draw along something that should be level or upright (or Ctrl+drag)"
    >
      Level by drawing a line
    </button>
  </div>

  <div class="group row">
    <button class="tool icon" onclick={() => rotate(-1)} title="Rotate left ([)">
      <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
        <path d="M3.5 6.5A5 5 0 1 1 4 11M3.5 2.5v4h4" />
      </svg>
      Left
    </button>
    <button class="tool icon" onclick={() => rotate(1)} title="Rotate right (])">
      <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
        <path d="M12.5 6.5A5 5 0 1 0 12 11M12.5 2.5v4h-4" />
      </svg>
      Right
    </button>
    <button class="tool icon" onclick={() => set(flip(crop))} title="Flip horizontally">
      <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
        <path d="M8 1.5v13M6 4 2 12h4ZM10 4l4 8h-4Z" />
      </svg>
      Flip
    </button>
  </div>

  <p class="hint">
    Drag the corners or edges to crop, inside to move, outside to rotate. The arrow keys move the
    crop too, and resize it with Shift.
  </p>

  <div class="actions">
    <button class="link" onclick={reset}>Reset</button>
    <span class="spacer"></span>
    <button class="ghost" onclick={() => session.cancelCrop()} title="Esc">Cancel</button>
    <button class="primary" onclick={() => session.finishCrop()} title="Enter">Done</button>
  </div>
</div>

<style>
  .panel {
    display: grid;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-4);
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
  }
  .title {
    font-weight: 500;
  }
  .size {
    font-family: var(--font-num);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .group {
    display: grid;
    gap: var(--space-2);
  }
  .row {
    grid-template-columns: repeat(3, 1fr);
  }
  .sub {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.06em;
    color: var(--text-2);
  }
  svg {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .aspect-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
  }
  .orient {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--bg-0);
  }
  .orient button {
    display: grid;
    place-items: center;
    width: 30px;
    height: 24px;
    border-radius: var(--radius-s);
    color: var(--text-2);
  }
  .orient button.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .orient button:disabled {
    opacity: 0.35;
  }
  .aspects {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: var(--space-1);
  }
  .aspects button {
    display: grid;
    justify-items: center;
    gap: 2px;
    padding: 8px 0 6px;
    border-radius: var(--radius-m);
    background: var(--bg-2);
    font-size: var(--fs-xs);
    color: var(--text-2);
    transition: background 100ms var(--ease);
  }
  .aspects button:hover {
    background: var(--bg-3);
    color: var(--text-1);
  }
  .aspects button.active {
    background: var(--bg-4, var(--line-strong));
    color: var(--text-0);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .aspects svg rect {
    transition: all 150ms var(--ease);
  }
  .aspects .dashed {
    stroke-dasharray: 2.5 2;
  }
  .tool.icon {
    display: flex;
    align-items: center;
    justify-content: center;
    gap: 6px;
  }
  .tool {
    padding: 5px 8px;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    color: var(--text-1);
    font-size: var(--fs-xs);
  }
  .tool:hover,
  .tool.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .hint {
    margin: 0;
    font-size: var(--fs-xs);
    color: var(--text-2);
    line-height: 1.4;
  }
  .actions {
    display: flex;
    align-items: center;
    gap: var(--space-2);
  }
  .spacer {
    flex: 1;
  }
  .actions button {
    padding: 6px 14px;
    border-radius: var(--radius-m);
    font-weight: 500;
  }
  .link {
    padding: 0 !important;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .link:hover {
    color: var(--text-0);
  }
  .primary {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .primary:hover {
    background: var(--line-strong);
  }
  .ghost {
    color: var(--text-1);
  }
</style>
