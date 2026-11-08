<script lang="ts">
  import Adjustments from "./Adjustments.svelte";
  import Canvas from "./Canvas.svelte";
  import HistoryPanel from "$lib/history/HistoryPanel.svelte";
  import { library } from "$lib/library/library.svelte";
  import { session } from "./session.svelte";
  import type { EditState } from "$lib/bindings/EditState";
  import type { Histogram } from "./renderer";

  let historyOpen = $state(true);
  let before = $state(false);
  let beforeEdit = $state<EditState | null>(null);
  let histogram = $state.raw<Histogram | null>(null);
  $effect(() => {
    session.defaultEdit().then((d) => (beforeEdit = d));
  });

  function onKeyDown(e: KeyboardEvent) {
    if (e.target instanceof HTMLInputElement) return;
    if (e.key === "\\" && !e.repeat) before = true;
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "o") {
      e.preventDefault();
      library.importFiles({ open: true });
    } else if (mod && e.key.toLowerCase() === "z") {
      e.preventDefault();
      if (e.shiftKey) session.redo();
      else session.undo();
    } else if (mod && e.key.toLowerCase() === "y") {
      e.preventDefault();
      session.redo();
    }
  }
  function onKeyUp(e: KeyboardEvent) {
    if (e.key === "\\") before = false;
  }

</script>

<svelte:window onkeydown={onKeyDown} onkeyup={onKeyUp} onblur={() => (before = false)} />

<div class="edit" class:history-closed={!historyOpen}>
  <aside class="history">
    {#if session.history}
      <HistoryPanel view={session.history} onhide={() => (historyOpen = false)} />
    {:else}
      <div class="panel-head">
        <span>History</span>
        <button class="icon" onclick={() => (historyOpen = false)} title="Hide history">‹</button>
      </div>
      <p class="hint">Open an image to see its history.</p>
    {/if}
  </aside>

  <section class="canvas-area">
    {#if session.image && session.pixels && session.edit && beforeEdit}
      <Canvas
        image={session.image}
        pixels={session.pixels}
        edit={session.edit}
        {before}
        {beforeEdit}
        onhistogram={(h) => (histogram = h)}
      />
      <div class="caption">
        <span>{session.image.name}</span>
        <span class="dim">{session.image.make} {session.image.model}</span>
      </div>
    {:else if !session.loading}
      <div class="empty">
        <button class="primary" onclick={() => library.importFiles({ open: true })}>Import Photos…</button>
        <p class="hint">Ctrl+O, or drop RAW files here. They’re copied into your library.</p>
      </div>
    {/if}

    {#if session.loading}
      <div class="status">{session.loading}</div>
    {/if}
    {#if session.error}
      <div class="status error">{session.error}</div>
    {/if}

    {#if !historyOpen}
      <button class="reveal" onclick={() => (historyOpen = true)} title="Show history">›</button>
    {/if}
  </section>

  <aside class="adjust">
    {#if session.image && session.edit}
      <Adjustments
        bind:edit={session.edit}
        image={session.image}
        {histogram}
        oncommit={(key) => session.commit(key)}
      />
    {/if}
  </aside>
</div>

<style>
  .edit {
    display: grid;
    grid-template-columns: 240px 1fr var(--panel-w);
    height: 100%;
    transition: grid-template-columns 180ms var(--ease);
  }
  .edit.history-closed {
    grid-template-columns: 0 1fr var(--panel-w);
  }
  .history {
    overflow: hidden;
    border-right: 1px solid var(--line);
    background: var(--bg-1);
    white-space: nowrap;
    min-height: 0;
  }
  .panel-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: var(--space-3) var(--space-4);
    font-weight: 500;
  }
  .icon,
  .reveal {
    color: var(--text-2);
    padding: 0 4px;
  }
  .icon:hover,
  .reveal:hover {
    color: var(--text-0);
  }
  .reveal {
    position: absolute;
    left: var(--space-2);
    top: var(--space-2);
  }
  .canvas-area {
    position: relative;
    display: grid;
    place-items: center;
    background: var(--bg-0);
    min-width: 0;
    overflow: hidden;
  }
  .caption {
    position: absolute;
    bottom: 4px;
    left: 0;
    right: 0;
    display: flex;
    justify-content: center;
    gap: var(--space-3);
    font-size: var(--fs-xs);
    color: var(--text-1);
    pointer-events: none;
  }
  .dim,
  .hint {
    color: var(--text-2);
    font-size: var(--fs-s);
  }
  .hint {
    padding: 0 var(--space-4);
  }
  .empty {
    text-align: center;
  }
  .primary {
    padding: 8px 18px;
    border-radius: var(--radius-m);
    background: var(--bg-3);
    color: var(--text-0);
    font-weight: 500;
  }
  .primary:hover {
    background: var(--line-strong);
  }
  .status {
    position: absolute;
    color: var(--text-1);
    font-size: var(--fs-s);
  }
  .status.error {
    bottom: var(--space-5);
    color: var(--danger);
  }
  .adjust {
    border-left: 1px solid var(--line);
    background: var(--bg-1);
    min-height: 0;
  }
</style>
