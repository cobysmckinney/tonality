<script lang="ts">
  import { app } from "$lib/app.svelte";
  import TopBar from "$lib/TopBar.svelte";
  import Library from "$lib/library/Library.svelte";
  import Edit from "$lib/edit/Edit.svelte";
  import { session } from "$lib/edit/session.svelte";
  import { api } from "$lib/api";

  api.startupPath().then((p) => {
    if (p) {
      app.mode = "edit";
      session.open(p);
    }
  });

  function onKey(e: KeyboardEvent) {
    if (e.target instanceof HTMLInputElement || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key === "g") app.mode = "library";
    else if (e.key === "e") app.mode = "edit";
  }
</script>

<svelte:window onkeydown={onKey} />

<div class="shell">
  <TopBar />
  <main>
    {#if app.mode === "library"}
      <Library />
    {:else}
      <Edit />
    {/if}
  </main>
</div>

<style>
  .shell {
    display: grid;
    grid-template-rows: var(--topbar-h) 1fr;
    height: 100%;
  }
  main {
    min-height: 0;
  }
</style>
