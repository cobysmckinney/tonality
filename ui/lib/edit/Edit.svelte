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
  import { compareLabels, type Compare } from "./compare";
  import { wbPick } from "./wbpick.svelte";
  import { untrack } from "svelte";
  import type { EditState } from "$lib/bindings/EditState";
  import type { Histogram } from "./renderer";
  import { onMount } from "svelte";
  import { palette, shortcut } from "$lib/palette/commands.svelte";
  import CullNote from "./CullNote.svelte";
  import { flagNote, ratingNote } from "$lib/library/culling";
  import { focusedControlOwnsKey } from "$lib/keys";

  const FLAG_KEYS: Record<string, number> = { p: 1, x: -1, u: 0 };
  let cullNote = $state<ReturnType<typeof CullNote> | null>(null);

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
  // "Level by drawing a line" belongs to one visit to crop mode. Whatever ends that visit (Done,
  // Cancel, Enter, Esc, the palette, another photo) switches it off, so the next crop doesn't
  // start with every drag drawing a line.
  $effect(() => {
    if (!session.cropping) level = false;
  });
  // The eyedropper ends when another photo opens or crop or mask mode starts.
  $effect(() => {
    void [session.image?.id, session.cropping, session.masking];
    untrack(() => wbPick.stop());
  });

  function onKeyDown(e: KeyboardEvent) {
    // A key the focused control uses is that control's: text in a field, Enter and Space on a
    // button reached with Tab (Enter on the crop panel's Cancel must not apply the crop), arrows
    // on a slider or on the panel's resize handle. A checkbox only uses Space, so ticking one
    // doesn't switch every shortcut off until something else is clicked.
    if (focusedControlOwnsKey(e.target, e.key)) return;
    if (session.cropping) {
      // Crop mode owns the keyboard: Enter applies, Esc cancels, [ ] rotate.
      const mod = e.ctrlKey || e.metaKey;
      if (e.key === "Enter") {
        e.preventDefault();
        session.finishCrop();
      } else if (e.key === "Escape") {
        session.cancelCrop();
      } else if ((e.key === "r" || e.key === "R") && !mod) {
        // Not while held: the repeat would start the next crop.
        if (!e.repeat) session.finishCrop();
      } else if ((e.key === "[" || e.key === "]") && session.edit) {
        session.edit.crop = turn(session.edit.crop, e.key === "]" ? 1 : -1);
        if (session.cropAspect) session.cropAspect = 1 / session.cropAspect;
      } else if (mod) e.preventDefault(); // no undo/zoom mid-crop
      return;
    }
    if ((e.key === "r" || e.key === "R") && !e.ctrlKey && !e.metaKey && !e.altKey && session.edit) {
      if (!e.repeat) session.startCrop();
      return;
    }
    const plain = !e.ctrlKey && !e.metaKey && !e.altKey;
    if ((e.key === "w" || e.key === "W") && plain && !session.masking && !e.repeat) {
      wbPick.toggle();
      return;
    }
    if (e.key === "Escape" && wbPick.active) {
      wbPick.stop();
      return;
    }
    if ((e.key === "m" || e.key === "M") && plain && session.edit) {
      // One press, one switch: held, it would flip mask mode on and off.
      if (e.repeat) return;
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
        // One press deletes one mask. The selection moves to its neighbour afterwards, so a
        // key held a moment too long would otherwise take that one too, and the next.
        if (!e.repeat) session.deleteMask(sel);
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
    // 0–5 rate, P / X / U pick, reject, unflag the open photo, as in the Library.
    if (plain && !e.repeat && session.image) {
      const note = cull(e.key, session.image.id);
      if (note) {
        cullNote?.show(note);
        return;
      }
    }
    // ← / → switch photos (a slider or a resize handle with focus has already kept them).
    if ((e.key === "ArrowLeft" || e.key === "ArrowRight") && plain && !e.shiftKey) {
      e.preventDefault();
      library.step(e.key === "ArrowLeft" ? -1 : 1);
      return;
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
  /** Rate or flag photo `id` for a culling key; returns the note to show, or null. */
  function cull(key: string, id: number): string | null {
    if (/^[0-5]$/.test(key)) {
      const r = library.setRating(Number(key), [id]);
      return r === null ? null : ratingNote(r);
    }
    const flag = FLAG_KEYS[key.toLowerCase()];
    if (flag === undefined) return null;
    library.setFlag(flag, [id]);
    return flagNote(flag);
  }

  /** What Canvas shows beside the photo: another version of it, or another photo. */
  function compareProp(c: Compare, current: string, name: string) {
    const l = compareLabels(c, current, name);
    const photo = c.kind === "photo" ? { image: c.image, pixels: c.pixels } : {};
    return { label: l.other, current: l.main, state: c.state, ...photo };
  }

  function onKeyUp(e: KeyboardEvent) {
    if (e.key === "\\") before = false;
  }

  // The command palette's actions that live in this component's state.
  onMount(() =>
    palette.provide({
      "view.zoomFit": () => canvas?.setZoom(null),
      "view.zoom100": () => canvas?.setZoom(1),
      "view.zoomIn": () => canvas?.zoomBy(Math.SQRT2),
      "view.zoomOut": () => canvas?.zoomBy(Math.SQRT1_2),
      "view.zoomToggle": () => canvas?.toggleZoom(),
      "crop.apply": () => session.finishCrop(),
      "crop.cancel": () => session.cancelCrop(),
      "view.filmstrip": {
        run: toggleStrip,
        title: () => (stripOpen ? "Hide filmstrip" : "Show filmstrip"),
      },
      "view.versions": {
        run: () => (historyOpen = !historyOpen),
        title: () => (historyOpen ? "Hide versions panel" : "Show versions panel"),
      },
    }),
  );
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
            ? compareProp(session.compare, session.history.head.branch, session.image.name)
            : null}
          oncloseCompare={() => session.stopCompare()}
          cropping={session.cropping}
          cropAspect={session.cropAspect}
          {level}
          oncrop={(c) => session.edit && (session.edit.crop = c)}
          onlevel={() => (level = false)}
          onhistogram={(h) => (histogram = h)}
          onpick={wbPick.active ? (at) => wbPick.pick(at) : null}
        />
        {#if session.preview && !before}
          <div class="preview-badge">Preview · {session.preview.label}</div>
        {/if}
        {#if session.masking && session.masks.length && !session.cropping}
          <MaskList />
        {/if}
      {:else if !session.loading}
        <div class="empty">
          <button class="btn primary" onclick={() => library.importFiles({ open: true })}>
            Import Photos…
          </button>
          <p class="hint">Ctrl+O, or drop RAW files here. They’re copied into your library.</p>
          <p class="hint">
            {shortcut("Mod+K")} finds any command; ? lists the keyboard shortcuts.
          </p>
        </div>
      {/if}

      {#if session.loading}
        <div class="status">{session.loading}</div>
      {/if}
      <CullNote bind:this={cullNote} />

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
        <span class="strip-label">
          {#if session.image}
            <span class="name">{session.image.name}</span>
            <span class="dim">{session.image.make} {session.image.model}</span>
          {:else}Filmstrip{/if}
        </span>
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
    display: flex;
    gap: var(--space-2);
    padding: 7px 36px 7px var(--space-3);
    font-size: var(--fs-xs);
    color: var(--text-2);
    white-space: nowrap;
  }
  .strip-label .name {
    color: var(--text-1);
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .strip-label .dim {
    flex: none;
  }
  .strip-toggle {
    position: absolute;
    top: 3px;
    right: 4px;
    width: 24px;
    height: 24px;
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
  .hint {
    color: var(--text-2);
    font-size: var(--fs-s);
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
  .adjust {
    min-height: 0;
  }
</style>
