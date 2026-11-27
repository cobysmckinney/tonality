<!--
  Slider: label + numeric readout + track.
  - Drag anywhere on the track. Hold Shift for fine (10x) control.
  - Double-click resets to `defaultValue`.
  - Arrow keys step; Shift+Arrow steps 10x.
  - `oninput` fires continuously, `oncommit` once per gesture (this is where history commits).
-->
<script lang="ts">
  interface Props {
    label: string;
    value: number;
    min: number;
    max: number;
    step?: number;
    defaultValue?: number;
    decimals?: number;
    unit?: string;
    /** Draw the fill from the default value (bipolar) instead of from `min`. */
    bipolar?: boolean;
    /** CSS background for the rail (e.g. a colour gradient). Replaces the fill. */
    track?: string;
    oninput?: (v: number) => void;
    oncommit?: (v: number) => void;
    /** Custom reset (double-click). Defaults to setting `defaultValue` and committing. */
    onreset?: () => void;
  }

  let {
    label,
    value,
    min,
    max,
    step = 1,
    defaultValue = 0,
    decimals = 0,
    unit = "",
    bipolar = true,
    track: trackBg,
    oninput,
    oncommit,
    onreset,
  }: Props = $props();

  let track: HTMLDivElement;
  let dragging = $state(false);
  let startX = 0;
  let startValue = 0;
  // The last press: a second one at the same place soon after is the second half of a
  // double-click (which resets), not another move of the slider.
  let lastPress: { x: number; at: number } | null = null;
  let second = false;
  const DOUBLE_CLICK_MS = 500;

  const span = $derived(max - min);
  const pct = (v: number) => ((v - min) / span) * 100;
  const fillFrom = $derived(bipolar ? pct(Math.min(Math.max(defaultValue, min), max)) : 0);
  const fillTo = $derived(pct(value));

  function clamp(v: number) {
    const snapped = Math.round(v / step) * step;
    return Math.min(max, Math.max(min, Number(snapped.toFixed(6))));
  }

  function set(v: number) {
    const c = clamp(v);
    if (c !== value) oninput?.(c);
    return c;
  }

  function onPointerDown(e: PointerEvent) {
    if (e.button !== 0) return;
    track.setPointerCapture(e.pointerId);
    dragging = true;
    startX = e.clientX;
    second =
      lastPress !== null &&
      e.timeStamp - lastPress.at < DOUBLE_CLICK_MS &&
      Math.abs(e.clientX - lastPress.x) < 4;
    lastPress = { x: e.clientX, at: e.timeStamp };
    if (second) {
      startValue = value;
      return;
    }
    // Clicking jumps to position; subsequent drag is relative so Shift-fine works.
    const rect = track.getBoundingClientRect();
    startValue = set(min + ((e.clientX - rect.left) / rect.width) * span);
  }

  function onPointerMove(e: PointerEvent) {
    if (!dragging) return;
    const rect = track.getBoundingClientRect();
    const scale = e.shiftKey ? 0.1 : 1;
    set(startValue + ((e.clientX - startX) / rect.width) * span * scale);
  }

  function onPointerUp(e: PointerEvent) {
    if (!dragging) return;
    dragging = false;
    track.releasePointerCapture(e.pointerId);
    // The second press of a double-click moved nothing and has nothing to record, unless it
    // turned into a drag after all.
    if (second && value === startValue) return;
    oncommit?.(value);
  }

  function reset() {
    if (onreset) return onreset();
    set(defaultValue);
    oncommit?.(clamp(defaultValue));
  }

  function onKey(e: KeyboardEvent) {
    const mult = e.shiftKey ? 10 : 1;
    let next: number | null = null;
    if (e.key === "ArrowRight" || e.key === "ArrowUp") next = value + step * mult;
    else if (e.key === "ArrowLeft" || e.key === "ArrowDown") next = value - step * mult;
    else if (e.key === "Home") next = min;
    else if (e.key === "End") next = max;
    if (next === null) return;
    e.preventDefault();
    oncommit?.(set(next));
  }

  const display = $derived(
    (value > 0 && bipolar && min < 0 ? "+" : "") + value.toFixed(decimals) + unit,
  );
</script>

<div class="slider" class:dragging>
  <div class="row">
    <span class="label">{label}</span>
    <span class="value" class:changed={value !== defaultValue}>{display}</span>
  </div>
  <div
    class="track"
    bind:this={track}
    role="slider"
    tabindex="0"
    aria-label={label}
    aria-valuemin={min}
    aria-valuemax={max}
    aria-valuenow={value}
    onpointerdown={onPointerDown}
    onpointermove={onPointerMove}
    onpointerup={onPointerUp}
    onpointercancel={onPointerUp}
    ondblclick={reset}
    onkeydown={onKey}
  >
    <div class="rail" class:tinted={!!trackBg} style:background={trackBg}></div>
    {#if !trackBg}
      <div
        class="fill"
        style:left="{Math.min(fillFrom, fillTo)}%"
        style:width="{Math.abs(fillTo - fillFrom)}%"
      ></div>
    {/if}
    <div class="thumb" style:left="{fillTo}%"></div>
  </div>
</div>

<style>
  .slider {
    padding: 1px 0;
  }
  .row {
    display: flex;
    justify-content: space-between;
    align-items: baseline;
    font-size: var(--fs-s);
    margin-bottom: 1px;
  }
  .label {
    color: var(--text-1);
  }
  .value {
    font-family: var(--font-num);
    font-size: var(--fs-xs);
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .value.changed {
    color: var(--text-0);
  }
  .track {
    position: relative;
    height: 16px;
    cursor: ew-resize;
    touch-action: none;
  }
  .rail,
  .fill {
    position: absolute;
    top: 7px;
    height: 2px;
    border-radius: 1px;
  }
  .rail {
    left: 0;
    right: 0;
    background: var(--bg-3);
  }
  .rail.tinted {
    top: 6px;
    height: 4px;
    border-radius: 2px;
  }
  .fill {
    background: var(--text-1);
  }
  .thumb {
    position: absolute;
    top: 3px;
    width: 10px;
    height: 10px;
    margin-left: -5px;
    border-radius: 50%;
    background: var(--text-0);
    box-shadow: 0 0 0 2px var(--bg-1);
    transition: transform 100ms var(--ease);
  }
  .track:hover .thumb,
  .dragging .thumb {
    transform: scale(1.2);
  }
</style>
