<!--
  The tools under the histogram, as a row of icon buttons: each names itself and its shortcut in
  its tooltip, and shows when it's on. Room for more (healing).
-->
<script lang="ts">
  import { session } from "./session.svelte";
  import { wbPick } from "./wbpick.svelte";

  interface Props {
    /** Masks on the photo, shown as a count on the Masks button. */
    masks: number;
  }
  let { masks }: Props = $props();

  const tools = $derived([
    {
      id: "crop",
      name: "Crop",
      tip: "Crop, rotate and straighten (R)",
      on: session.cropping,
      icon: "M4 1v11h11M1 4h11v11",
      run: () => session.startCrop(),
    },
    {
      id: "masks",
      name: "Masks",
      tip: "Masks: gradients, brush and AI selections (M)",
      on: session.masking,
      icon: "M8 2a6 6 0 1 0 0 12A6 6 0 0 0 8 2Zm0 0v12",
      run: () => (session.masking ? session.stopMasks() : session.startMasks()),
    },
    {
      id: "wb",
      name: "White balance picker",
      tip: "White balance picker (W): click something that should be neutral grey",
      on: wbPick.active,
      icon: "M9 3.5 12.5 7M10.8 1.7a1.6 1.6 0 0 1 2.3 0l1.2 1.2a1.6 1.6 0 0 1 0 2.3L12 7.5 8.5 4ZM10.3 5.7 4.5 11.5 3 14l-1-1 2.5-1.5",
      run: () => wbPick.toggle(),
    },
  ]);
</script>

<div class="strip" role="toolbar" aria-label="Tools">
  {#each tools as t (t.id)}
    <button
      class="tool"
      class:on={t.on}
      aria-label={t.name}
      aria-pressed={t.on}
      title={t.tip}
      onclick={t.run}
    >
      <svg viewBox="0 0 16 16" width="16" height="16" aria-hidden="true"><path d={t.icon} /></svg>
      {#if t.id === "masks" && masks > 0}<span class="count">{masks}</span>{/if}
    </button>
  {/each}
</div>

<style>
  .strip {
    display: flex;
    gap: var(--space-1);
    padding: 0 var(--space-4);
  }
  .tool {
    position: relative;
    display: grid;
    place-items: center;
    width: 36px;
    height: 30px;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    color: var(--text-1);
    transition:
      background 120ms var(--ease),
      color 120ms var(--ease);
  }
  .tool:hover {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .tool.on {
    background: var(--text-0);
    color: var(--bg-0);
  }
  svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .count {
    position: absolute;
    top: 2px;
    right: 2px;
    min-width: 13px;
    padding: 0 3px;
    border-radius: var(--radius-pill);
    background: var(--bg-4);
    color: var(--text-0);
    font-family: var(--font-num);
    font-size: 9px;
    line-height: 13px;
    text-align: center;
  }
</style>
