<script lang="ts">
  // Crop & rotate controls, shown in place of the adjustments while cropping.
  import type { Crop } from "$lib/bindings/Crop";
  import type { ImageInfo } from "$lib/bindings/ImageInfo";
  import Slider from "$lib/design/Slider.svelte";
  import { ASPECTS, FULL, flip, shrinkToFit, turn, turnedSize, withAspect, type Rect } from "./crop";
  import { session } from "./session.svelte";

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
  let portrait = $state(false);

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
    if (ratio !== null) set({ ...crop, rect: withAspect(crop.rect as Rect, ratio, crop.angle, ...turned) });
  }

  function swap() {
    portrait = !portrait;
    apply();
  }

  // ---- straighten ----
  let base: Rect | null = null;
  function straighten(angle: number) {
    base ??= [...(crop.rect as Rect)] as Rect;
    set({ ...crop, angle, rect: shrinkToFit(base, angle, ...turned) });
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
    <span class="sub">Aspect</span>
    <div class="aspects">
      {#each ASPECTS as a (a.label)}
        <button class:active={preset === a.label} onclick={() => choose(a.label)}>{a.label}</button>
      {/each}
      <button class="swap" onclick={swap} disabled={preset === "Free"} title="Swap portrait / landscape">⇄</button>
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
      oncommit={() => (base = null)}
      onreset={() => {
        base = null;
        straighten(0);
        base = null;
      }}
    />
    <button class="tool" class:active={level} onclick={ontogglelevel} title="Draw along something that should be level or upright (or Ctrl+drag)">
      Level by drawing a line
    </button>
  </div>

  <div class="group row">
    <button class="tool" onclick={() => rotate(-1)} title="Rotate left ([)">⟲ Left</button>
    <button class="tool" onclick={() => rotate(1)} title="Rotate right (])">⟳ Right</button>
    <button class="tool" onclick={() => set(flip(crop))} title="Flip horizontally">⇋ Flip</button>
  </div>

  <p class="hint">Drag the corners or edges to crop, inside to move, outside to rotate.</p>

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
  .aspects {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--bg-0);
  }
  .aspects button {
    padding: 4px 0;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .aspects button.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .aspects button:disabled {
    opacity: 0.35;
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
