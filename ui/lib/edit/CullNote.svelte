<!--
  A brief, quiet note on the canvas after a rating or flag key ("★★★", "Picked"), since the
  filmstrip that shows them can be hidden. Fades out after a second; a new press replaces it.
-->
<script lang="ts">
  import { onDestroy } from "svelte";
  import { fade } from "svelte/transition";

  const SHOW_MS = 1000;

  let text = $state<string | null>(null);
  let timer: ReturnType<typeof setTimeout> | undefined;

  export function show(t: string) {
    text = t;
    clearTimeout(timer);
    timer = setTimeout(() => (text = null), SHOW_MS);
  }

  onDestroy(() => clearTimeout(timer));
</script>

{#if text !== null}
  <div class="note" role="status" out:fade={{ duration: 250 }}>{text}</div>
{/if}

<style>
  .note {
    position: absolute;
    top: var(--space-4);
    left: 50%;
    transform: translateX(-50%);
    padding: 4px 14px;
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, var(--bg-1) 85%, transparent);
    color: var(--text-0);
    font-size: var(--fs-s);
    letter-spacing: 1px;
    white-space: nowrap;
    pointer-events: none;
    z-index: 3;
  }
</style>
