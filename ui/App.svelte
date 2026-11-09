<script lang="ts">
  import { app } from "$lib/app.svelte";
  import TopBar from "$lib/TopBar.svelte";
  import Library from "$lib/library/Library.svelte";
  import Edit from "$lib/edit/Edit.svelte";
  import { api } from "$lib/api";
  import { library } from "$lib/library/library.svelte";
  import ExportDialog from "$lib/export/ExportDialog.svelte";
  import { exportCurrent } from "$lib/export/targets";

  // Dev hooks: TONALITY_OPEN imports and opens a file; TONALITY_IMPORT imports a folder.
  api.startupPath().then((p) => {
    if (p) {
      app.mode = "edit";
      library.importPaths([p], { open: true });
    }
  });

  api.startupImport().then((p) => {
    if (p) library.importPaths([p]);
  });

  function onKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "e") {
      e.preventDefault();
      exportCurrent();
      return;
    }
    if (e.target instanceof HTMLInputElement || e.ctrlKey || e.metaKey || e.altKey) return;
    if (e.key === "g") app.mode = "library";
    else if (e.key === "e") {
      // From the Library, E opens the focused photo.
      if (app.mode === "library" && library.focus !== null) library.open();
      else app.mode = "edit";
    }
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
<ExportDialog />

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
