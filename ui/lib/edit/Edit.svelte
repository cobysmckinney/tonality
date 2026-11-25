<script lang="ts">
  import Adjustments from "./Adjustments.svelte";
  import Canvas from "./Canvas.svelte";
  import HistoryPanel from "$lib/history/HistoryPanel.svelte";
  import ApplyDialog from "$lib/history/ApplyDialog.svelte";
  import { historyWidth } from "$lib/history/width.svelte";
  import CropPanel from "./CropPanel.svelte";
  import MaskPanel from "./MaskPanel.svelte";
  import MaskList from "./MaskList.svelte";
  import Filmstrip from "./Filmstrip.svelte";
  import { turn } from "./crop";
  import { library } from "$lib/library/state.svelte";
  import { session } from "./session.svelte";
  import type { EditState } from "$lib/bindings/EditState";
  import type { Histogram } from "./renderer";

  let historyOpen = $state(true);
  let before = $state(false);
  let beforeEdit = $state<EditState | null>(null);
  let histogram = $state.raw<Histogram | null>(null);
  let canvas = $state<ReturnType<typeof Canvas> | null>(null);
  let level = $state(false);

  const STRIP_KEY = "tonality.edit.filmstrip";
  let stripOpen = $state(true);
  try {
    stripOpen = localStorage.getItem(STRIP_KEY) !== "0";
  } catch {
    /* per-viewer convenience only */
  }
  function toggleStrip() {
    stripOpen = !stripOpen;
    try {
      localStorage.setItem(STRIP_KEY, stripOpen ? "1" : "0");
    } catch {
      /* ignore */
    }
  }
  $effect(() => {
    session.defaultEdit().then((d) => (beforeEdit = d));
  });

  function onKeyDown(e: KeyboardEvent) {
    if (e.target instanceof HTMLInputElement) return;
    if (session.cropping) {
      // Crop mode owns the keyboard: Enter applies, Esc cancels, [ ] rotate.
      const mod = e.ctrlKey || e.metaKey;
      if (e.key === "Enter") {
        e.preventDefault();
        level = false;
        session.finishCrop();
      } else if (e.key === "Escape") {
        level = false;
        session.cancelCrop();
      } else if ((e.key === "r" || e.key === "R") && !mod) {
        level = false;
        session.finishCrop();
      } else if ((e.key === "[" || e.key === "]") && session.edit) {
        session.edit.crop = turn(session.edit.crop, e.key === "]" ? 1 : -1);
        if (session.cropAspect) session.cropAspect = 1 / session.cropAspect;
      } else if (mod) e.preventDefault(); // no undo/zoom mid-crop
      return;
    }
    if ((e.key === "r" || e.key === "R") && !e.ctrlKey && !e.metaKey && !e.altKey && session.edit) {
      session.startCrop();
      return;
    }
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    if ((e.key === "m" || e.key === "M") && plain && session.edit) {
      if (session.masking) session.stopMasks();
      else session.startMasks();
      return;
    }
    if (session.masking && plain) {
      // Mask mode: O overlay, Delete removes the selected mask, [ ] brush size, Esc leaves.
      const sel = session.selected;
      if (e.key === "o" || e.key === "O") {
        session.maskOverlay = !session.maskOverlay;
        return;
      }
      if ((e.key === "Delete" || e.key === "Backspace") && sel !== null) {
        session.deleteMask(sel);
        return;
      }
      if (e.key === "[" || e.key === "]") {
        const b = session.brush;
        b.size = Math.min(0.3, Math.max(0.003, b.size * (e.key === "]" ? 1.2 : 1 / 1.2)));
        return;
      }
      if (e.key === "Escape" && !session.compare) {
        session.stopMasks();
        return;
      }
    }
    // ← / → switch photos (unless a slider has focus: there they nudge its value).
    if ((e.key === "ArrowLeft" || e.key === "ArrowRight") && plain && !e.shiftKey) {
      if (!(e.target instanceof Element && e.target.closest("[role=slider]"))) {
        e.preventDefault();
        library.step(e.key === "ArrowLeft" ? -1 : 1);
        return;
      }
    }
    if (e.key === "\\" && !e.repeat) before = true;
    if (e.key === "Escape" && session.compare) {
      session.stopCompare();
      return;
    }
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
    } else if (mod && e.key === "0") {
      e.preventDefault();
      canvas?.setZoom(null);
    } else if (mod && e.key === "1") {
      e.preventDefault();
      canvas?.setZoom(1);
    } else if (mod && (e.key === "=" || e.key === "+")) {
      e.preventDefault();
      canvas?.zoomBy(Math.SQRT2);
    } else if (mod && e.key === "-") {
      e.preventDefault();
      canvas?.zoomBy(Math.SQRT1_2);
    } else if (!mod && !e.altKey && (e.key === "z" || e.key === "Z") && !e.repeat) {
      canvas?.toggleZoom();
    }
  }
  function onKeyUp(e: KeyboardEvent) {
    if (e.key === "\\") before = false;
  }
</script>

<svelte:window onkeydown={onKeyDown} onkeyup={onKeyUp} onblur={() => (before = false)} />

<ApplyDialog />

<div
  class="edit"
  class:history-closed={!historyOpen}
  class:resizing={historyWidth.dragging}
  style:--history-w="{historyWidth.px}px"
>
  <!-- Crop mode: no undo or checkout until Done / Cancel (Cancel restores the crop it entered with). -->
  <aside class="history surface" inert={session.cropping} class:inert={session.cropping}>
    {#if session.history}
      <HistoryPanel view={session.history} onhide={() => (historyOpen = false)} />
    {:else}
      <div class="panel-head">
        <span class="eyebrow">Versions</span>
        <button class="icon-btn" onclick={() => (historyOpen = false)} title="Hide panel">
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path d="M10 3 5 8l5 5" />
          </svg>
        </button>
      </div>
      <p class="hint">Open a photo to see its versions and history.</p>
    {/if}
  </aside>

  <div class="center">
    <section class="canvas-area">
      {#if session.image && session.pixels && session.edit && beforeEdit}
        <Canvas
          bind:this={canvas}
          image={session.image}
          pixels={session.pixels}
          edit={session.preview?.state ?? session.edit}
          {before}
          {beforeEdit}
          compare={session.compare && session.history
            ? {
                label: session.compare.branch,
                current: session.history.head.branch,
                state: session.compare.state,
              }
            : null}
          oncloseCompare={() => session.stopCompare()}
          cropping={session.cropping}
          cropAspect={session.cropAspect}
          {level}
          oncrop={(c) => session.edit && (session.edit.crop = c)}
          onlevel={() => (level = false)}
          onhistogram={(h) => (histogram = h)}
        />
        {#if session.preview && !before}
          <div class="preview-badge">Preview · {session.preview.label}</div>
        {/if}
        {#if session.masking && session.masks.length && !session.cropping}
          <MaskList />
        {/if}
        <div class="caption">
          <span>{session.image.name}</span>
          <span class="dim">{session.image.make} {session.image.model}</span>
        </div>
      {:else if !session.loading}
        <div class="empty">
          <button class="btn primary" onclick={() => library.importFiles({ open: true })}>
            Import Photos…
          </button>
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
        <button
          class="reveal icon-btn"
          onclick={() => (historyOpen = true)}
          title="Show versions &amp; history"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path d="m6 3 5 5-5 5" />
          </svg>
        </button>
      {/if}
    </section>

    <div class="filmstrip surface" class:closed={!stripOpen}>
      {#if stripOpen}
        <Filmstrip />
      {:else}
        <span class="strip-label">Filmstrip</span>
      {/if}
      <button
        class="icon-btn strip-toggle"
        onclick={toggleStrip}
        title={stripOpen ? "Hide filmstrip" : "Show filmstrip"}
      >
        <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
          <path d={stripOpen ? "m4 6 4 4 4-4" : "m4 10 4-4 4 4"} />
        </svg>
      </button>
    </div>
  </div>

  <aside class="adjust surface">
    {#if session.image && session.edit && session.cropping}
      <CropPanel image={session.image} {level} ontogglelevel={() => (level = !level)} />
    {:else if session.image && session.edit && session.masking}
      <MaskPanel {histogram} />
    {:else if session.image && session.edit}
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
    grid-template-columns: var(--history-w) 1fr var(--panel-w);
    height: 100%;
    transition: grid-template-columns 180ms var(--ease);
  }
  .edit.resizing {
    transition: none;
    cursor: col-resize;
  }
  .preview-badge {
    position: absolute;
    top: var(--space-3);
    left: 50%;
    transform: translateX(-50%);
    padding: 2px 10px;
    border-radius: var(--radius-pill);
    background: rgba(0, 0, 0, 0.6);
    font-size: var(--fs-xs);
    color: var(--text-1);
    pointer-events: none;
    z-index: 2;
  }
  .edit.history-closed {
    grid-template-columns: 0 1fr var(--panel-w);
  }
  .history {
    min-height: 0;
    white-space: nowrap;
  }
  .history-closed .history {
    border: none;
  }
  .history.inert {
    opacity: 0.5;
  }
  .panel-head {
    display: flex;
    justify-content: space-between;
    align-items: center;
    height: 44px;
    padding: 0 var(--space-2) 0 var(--space-4);
  }
  .reveal {
    position: absolute;
    left: var(--space-2);
    top: var(--space-2);
    background: var(--bg-1);
    z-index: 2;
  }
  .center {
    display: flex;
    flex-direction: column;
    gap: var(--gutter);
    min-width: 0;
    min-height: 0;
    margin: 0 var(--gutter);
  }
  .filmstrip {
    position: relative;
    flex: none;
    height: 116px;
    transition: height 160ms var(--ease);
  }
  .filmstrip.closed {
    height: 30px;
  }
  .strip-label {
    display: block;
    padding: 7px var(--space-3);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .strip-toggle {
    position: absolute;
    top: 3px;
    right: 4px;
    width: 24px;
    height: 24px;
  }
  .filmstrip :global(.keys) {
    margin-right: 28px;
  }
  .canvas-area {
    position: relative;
    flex: 1;
    min-height: 0;
    display: grid;
    place-items: center;
    border-radius: var(--radius-l);
    background: var(--bg-0);
    min-width: 0;
    overflow: hidden;
  }
  .caption {
    position: absolute;
    bottom: var(--space-3);
    left: 50%;
    transform: translateX(-50%);
    display: flex;
    gap: var(--space-3);
    padding: 3px 12px;
    border-radius: var(--radius-pill);
    background: color-mix(in srgb, var(--bg-1) 80%, transparent);
    font-size: var(--fs-xs);
    color: var(--text-1);
    white-space: nowrap;
    pointer-events: none;
  }
  .dim,
  .hint {
    color: var(--text-2);
    font-size: var(--fs-s);
  }
  .caption .dim {
    font-size: var(--fs-xs);
  }
  .hint {
    padding: 0 var(--space-4);
    white-space: normal;
  }
  .empty {
    display: grid;
    justify-items: center;
    gap: var(--space-3);
    text-align: center;
  }
  .empty .btn {
    height: 36px;
    padding: 0 var(--space-5);
  }
  .status {
    position: absolute;
    padding: 4px 12px;
    border-radius: var(--radius-pill);
    background: var(--bg-1);
    color: var(--text-1);
    font-size: var(--fs-s);
  }
  .status.error {
    bottom: var(--space-6);
    color: var(--danger);
  }
  .adjust {
    min-height: 0;
  }
</style>
