<script lang="ts">
  import { app, type Mode } from "$lib/app.svelte";
  import { exportCurrent, exportCount } from "$lib/export/targets";
  import BranchMenu from "$lib/history/BranchMenu.svelte";
  import Logo from "$lib/design/Logo.svelte";

  const modes: { id: Mode; label: string; key: string }[] = [
    { id: "library", label: "Library", key: "G" },
    { id: "edit", label: "Edit", key: "E" },
  ];
</script>

<header>
  <div class="left">
    <div class="brand" data-brand-wordmark style:visibility={app.splash ? "hidden" : null}>
      <Logo height={21} label="tonality" />
    </div>
    {#if app.mode === "edit"}<BranchMenu />{/if}
  </div>

  <nav class="seg modes" aria-label="Mode">
    {#each modes as m (m.id)}
      <button
        class:active={app.mode === m.id}
        onclick={() => (app.mode = m.id)}
        title="{m.label} ({m.key})"
      >
        {m.label}
      </button>
    {/each}
  </nav>

  <div class="right">
    <button
      class="btn"
      onclick={exportCurrent}
      disabled={!exportCount()}
      title="Export (Ctrl+Shift+E)"
    >
      <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
        ><path d="M8 10V2M5 5l3-3 3 3M3 9v4h10V9" /></svg
      >
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
    color: var(--text-0);
    /* The y's descender hangs below; lift the word so its x-height sits centred. */
    margin-top: 2px;
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
