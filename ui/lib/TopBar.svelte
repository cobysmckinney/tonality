<script lang="ts">
  import { app, type Mode } from "$lib/app.svelte";
  import { exportCurrent, exportCount } from "$lib/export/targets";
  import BranchMenu from "$lib/history/BranchMenu.svelte";

  const modes: { id: Mode; label: string; key: string }[] = [
    { id: "library", label: "Library", key: "G" },
    { id: "edit", label: "Edit", key: "E" },
  ];
</script>

<header>
  <div class="left">
    <div class="brand">
      <span class="mark" aria-hidden="true"></span>
      tonality
    </div>
    {#if app.mode === "edit"}<BranchMenu />{/if}
  </div>

  <nav class="seg modes" aria-label="Mode">
    {#each modes as m (m.id)}
      <button class:active={app.mode === m.id} onclick={() => (app.mode = m.id)} title="{m.label} ({m.key})">
        {m.label}
      </button>
    {/each}
  </nav>

  <div class="right">
    <button class="btn" onclick={exportCurrent} disabled={!exportCount()} title="Export (Ctrl+Shift+E)">
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"><path d="M8 10V2M5 5l3-3 3 3M3 9v4h10V9" /></svg>
      Export
    </button>
  </div>
</header>

<style>
  header {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 0 calc(var(--gutter) + var(--space-2));
  }
  .left {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    min-width: 0;
  }
  .brand {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text-0);
  }
  /* A tonal ramp: the app's whole idea in one glyph. */
  .mark {
    width: 14px;
    height: 14px;
    border-radius: 4px;
    background: linear-gradient(135deg, #f0f0f0, #5a5a5a 60%, #1c1c1c);
    box-shadow: inset 0 0 0 1px rgba(255, 255, 255, 0.12);
  }
  .modes {
    min-width: 180px;
  }
  .modes button {
    font-size: var(--fs-s);
    padding: 4px 16px;
  }
  .right {
    justify-self: end;
  }
</style>
