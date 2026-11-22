<script lang="ts">
  import { app } from "$lib/app.svelte";
  import TopBar from "$lib/TopBar.svelte";
  import Library from "$lib/library/Library.svelte";
  import Edit from "$lib/edit/Edit.svelte";
  import { api } from "$lib/api";
  import { library } from "$lib/library/state.svelte";
  import ExportDialog from "$lib/export/ExportDialog.svelte";
  import { exportCurrent } from "$lib/export/targets";
  import Splash from "$lib/Splash.svelte";
  import { targetOwnsKey } from "$lib/keys";
  import SettingsDialog from "$lib/settings/SettingsDialog.svelte";
  import { updates } from "$lib/settings/updates.svelte";

  updates.init();

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

  // Dev: TONALITY_DEV_SCRIPT=file.js drives the UI (debug builds only).
  if (import.meta.env.DEV) {
    api.devScript().then(async (src) => {
      if (!src) return;
      const { session } = await import("$lib/edit/session.svelte");
      const { snapshotNextFrame } = await import("$lib/edit/versions");
      const snapshot = async (name: string | null = null) =>
        api.devSaveSnapshot(await snapshotNextFrame(), name);
      const sleep = (ms: number) => new Promise((r) => setTimeout(r, ms));
      try {
        await new Function(
          "ctx",
          `return (async ({ app, api, library, session, sleep, snapshot }) => { ${src} })(ctx);`,
        )({
          app,
          api,
          library,
          session,
          sleep,
          snapshot,
        });
        console.info("[dev-script] done");
      } catch (e) {
        console.error("[dev-script]", e);
      }
    });
  }

  function onKey(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.shiftKey && e.key.toLowerCase() === "e") {
      e.preventDefault();
      exportCurrent();
      return;
    }
    if ((e.ctrlKey || e.metaKey) && e.key === ",") {
      e.preventDefault();
      updates.open = true;
      return;
    }
    if (targetOwnsKey(e.target, e.key) || e.ctrlKey || e.metaKey || e.altKey) return;
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
<SettingsDialog />
{#if app.splash}<Splash />{/if}

<style>
  .shell {
    display: grid;
    grid-template-rows: var(--topbar-h) 1fr;
    height: 100%;
  }
  main {
    min-height: 0;
    padding: 0 var(--gutter) var(--gutter);
  }
</style>
