<!--
  The masks, floating over the photo: each one as a black-and-white thumbnail of what it
  selects. Pick one to adjust it in the side panel; hover one to see its area in red.
-->
<script lang="ts">
  import type { Mask } from "$lib/bindings/Mask";
  import MaskCreate from "./MaskCreate.svelte";
  import { KIND_LABEL, MAX_MASKS, SHAPE_ICON, aiKey, maskId, shaderIndex } from "./masks";
  import { session } from "./session.svelte";

  const masks = $derived(session.masks);
  const sel = $derived(session.selected);
  const progress = $derived(session.aiProgress);
  const pct = (p: { done: number; total: number }) =>
    Math.round((100 * p.done) / Math.max(p.total, 1));

  let adding = $state(false);
  let collapsed = $state(false);

  function status(m: Mask) {
    const k = aiKey(m);
    return k ? session.aiStatus.get(k) : undefined;
  }

  function kindLabel(m: Mask) {
    return m.shape.kind === "ai" ? "AI" : KIND_LABEL[m.shape.kind];
  }

  function toggleShown(i: number) {
    const m = session.edit?.masks?.[i];
    if (!m) return;
    m.enabled = !m.enabled;
    session.commit(null);
  }

  // ---- rename ----
  let renaming = $state<number | null>(null);
  let draft = $state("");
  // Renames are by mask, not position: drop one if its mask goes away (a delete, undo).
  $effect(() => {
    if (renaming !== null && !masks.some((m) => maskId(m) === renaming)) renaming = null;
  });
  function startRename(m: Mask) {
    renaming = maskId(m);
    draft = m.name;
  }
  function finishRename() {
    const id = renaming;
    renaming = null;
    const m = session.edit?.masks?.find((x) => maskId(x) === id);
    const name = draft.trim();
    if (m && name && name !== m.name) {
      m.name = name;
      session.commit(null);
    }
  }
  function focus(el: HTMLInputElement) {
    el.focus();
    el.select();
  }

  /** Draws a thumbnail into its canvas, and again whenever it changes. */
  function thumb(canvas: HTMLCanvasElement, img: ImageData) {
    const draw = (d: ImageData) => {
      canvas.width = d.width;
      canvas.height = d.height;
      canvas.getContext("2d")?.putImageData(d, 0, 0);
    };
    draw(img);
    return { update: draw };
  }

  function onWindowPointerDown(e: PointerEvent) {
    if (adding && !(e.target instanceof Element && e.target.closest(".float"))) adding = false;
  }
  function onWindowKey(e: KeyboardEvent) {
    if (adding && e.key === "Escape") {
      e.stopPropagation();
      adding = false;
    }
  }
</script>

<svelte:window onpointerdown={onWindowPointerDown} onkeydowncapture={onWindowKey} />

<div
  class="float"
  role="region"
  aria-label="Masks"
  onpointerleave={() => (session.maskHover = null)}
>
  <header>
    <button
      class="title"
      onclick={() => (collapsed = !collapsed)}
      aria-expanded={!collapsed}
      title={collapsed ? "Show masks" : "Hide masks"}
    >
      <svg
        class="chev"
        class:open={!collapsed}
        viewBox="0 0 16 16"
        width="12"
        height="12"
        aria-hidden="true"
      >
        <path d="m6 4 4 4-4 4" />
      </svg>
      Masks
      <span class="count">{masks.length}</span>
    </button>
    <div class="add-wrap">
      <button
        class="icon"
        class:on={adding}
        onclick={() => (adding = !adding)}
        disabled={!adding && masks.length >= MAX_MASKS}
        aria-expanded={adding}
        title={adding
          ? "Cancel"
          : masks.length >= MAX_MASKS
            ? `Up to ${MAX_MASKS} masks per photo`
            : "Create a new mask"}
      >
        <svg
          class="plus"
          class:x={adding}
          viewBox="0 0 16 16"
          width="14"
          height="14"
          aria-hidden="true"
        >
          <path d="M8 3v10M3 8h10" />
        </svg>
      </button>
    </div>
  </header>

  {#if adding}
    <!-- In place of the list, so it always fits. -->
    <div class="menu"><MaskCreate onpicked={() => (adding = false)} /></div>
  {:else if !collapsed}
    <ul>
      {#each masks as m, i (maskId(m))}
        {@const st = status(m)}
        {@const img = session.maskThumbs[i]}
        {@const drawn = m.enabled && shaderIndex(masks, i) !== null}
        <li
          class:selected={sel === i}
          class:hidden={!m.enabled}
          onpointerenter={() => (session.maskHover = i)}
        >
          <button
            class="pick"
            aria-current={sel === i ? "true" : undefined}
            onclick={() => (session.maskIndex = i)}
            ondblclick={() => startRename(m)}
            onkeydown={(e) => {
              if (e.key !== "F2") return;
              e.preventDefault();
              startRename(m);
            }}
            title="{m.name}: double-click (or F2) to rename"
          >
            <span class="thumb">
              {#if drawn && img}
                <canvas use:thumb={img}></canvas>
              {:else if !drawn}
                <span
                  class="off"
                  title={m.enabled ? `Only the first ${MAX_MASKS} shown masks are drawn` : "Hidden"}
                >
                  <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
                    <path
                      d="M2 2l12 12M6.3 4c.5-.2 1.1-.4 1.7-.4 4.5 0 7 4.4 7 4.4s-.7 1.2-2 2.4M4 5.3C2.2 6.5 1 8 1 8s2.5 4.5 7 4.5c1 0 2-.2 2.8-.6"
                    />
                  </svg>
                </span>
              {/if}
              <span class="kind" title={kindLabel(m)}>
                <svg viewBox="0 0 16 16" width="10" height="10" aria-hidden="true">
                  <path d={SHAPE_ICON[m.shape.kind]} />
                </svg>
              </span>
            </span>
            {#if renaming !== maskId(m)}
              <span class="text">
                <span class="name">{m.name}</span>
                {#if st?.state === "working"}
                  <span class="state">
                    {progress?.stage === "download"
                      ? `Downloading ${pct(progress)}%`
                      : "Detecting…"}
                  </span>
                {:else if st?.state === "empty"}
                  <span class="state warn">Nothing found</span>
                {:else if st?.state === "error"}
                  <span class="state warn">Detection failed</span>
                {:else if m.enabled && !drawn}
                  <span class="state warn">Not drawn: over {MAX_MASKS}</span>
                {:else if m.invert}
                  <span class="state">Inverted</span>
                {/if}
              </span>
            {/if}
          </button>
          {#if renaming === maskId(m)}
            <!-- Over the row, not inside the button: an <input> in a <button> edits unreliably. -->
            <input
              class="rename"
              use:focus
              bind:value={draft}
              onblur={finishRename}
              onkeydown={(e) => {
                e.stopPropagation();
                if (e.key === "Enter") finishRename();
                if (e.key === "Escape") renaming = null;
              }}
            />
          {/if}
          <div class="actions">
            <button
              class="icon"
              onclick={() => toggleShown(i)}
              title={m.enabled ? "Hide this mask's effect" : "Show this mask's effect"}
            >
              {#if m.enabled}
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <path d="M1 8s2.5-4.5 7-4.5S15 8 15 8s-2.5 4.5-7 4.5S1 8 1 8Z" /><circle
                    cx="8"
                    cy="8"
                    r="2"
                  />
                </svg>
              {:else}
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <path
                    d="M2 2l12 12M6.3 4c.5-.2 1.1-.4 1.7-.4 4.5 0 7 4.4 7 4.4s-.7 1.2-2 2.4M4 5.3C2.2 6.5 1 8 1 8s2.5 4.5 7 4.5c1 0 2-.2 2.8-.6"
                  />
                </svg>
              {/if}
            </button>
            <button class="icon" onclick={() => session.deleteMask(i)} title="Delete (Del)">
              <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                <path d="M4 4l8 8M12 4l-8 8" />
              </svg>
            </button>
          </div>
        </li>
      {/each}
    </ul>

    <footer>
      <label title="Show the selected mask's area in red (O)">
        <input type="checkbox" bind:checked={session.maskOverlay} />
        Show overlay
        <kbd>O</kbd>
      </label>
    </footer>
  {/if}
</div>

<style>
  .float {
    position: absolute;
    /* Bottom right, growing upwards (clear of the zoom button at the top). */
    bottom: var(--space-3);
    right: var(--space-3);
    z-index: 3;
    width: 236px;
    max-height: calc(100% - 40px - var(--space-3));
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-l);
    border: 1px solid var(--line);
    background: color-mix(in srgb, var(--bg-1) 94%, transparent);
    backdrop-filter: blur(8px);
    box-shadow: 0 8px 24px rgb(0 0 0 / 0.35);
  }
  header {
    flex: none;
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: var(--space-1) var(--space-1) var(--space-1) var(--space-2);
  }
  .title {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 30px;
    padding: 0 var(--space-1);
    font-weight: 500;
    color: var(--text-0);
  }
  .chev {
    transition: transform 150ms var(--ease);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .count {
    font-size: var(--fs-xs);
    color: var(--text-2);
    font-weight: 400;
    font-variant-numeric: tabular-nums;
  }
  svg {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .icon {
    display: grid;
    place-items: center;
    width: 28px;
    height: 28px;
    border-radius: var(--radius-s);
    color: var(--text-2);
  }
  .icon:hover:not(:disabled),
  .icon.on {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .icon:disabled {
    opacity: 0.4;
  }
  .menu {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--space-1) var(--space-2);
  }
  .plus {
    transition: transform 150ms var(--ease);
  }
  .plus.x {
    transform: rotate(45deg);
  }
  ul {
    flex: 1 1 auto;
    min-height: 0;
    overflow-y: auto;
    list-style: none;
    margin: 0;
    padding: 0 var(--space-1);
    display: grid;
    align-content: start;
    gap: 2px;
  }
  li {
    position: relative;
    display: flex;
    align-items: center;
    border-radius: var(--radius-m);
  }
  li:hover {
    background: var(--bg-2);
  }
  li.selected {
    background: var(--bg-3);
  }
  .pick {
    flex: 1;
    min-width: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 4px;
    text-align: left;
  }
  .thumb {
    position: relative;
    flex: none;
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    border-radius: var(--radius-s);
    background: #000;
    overflow: hidden;
    box-shadow: inset 0 0 0 1px var(--line);
  }
  li.selected .thumb {
    box-shadow: 0 0 0 2px var(--text-1);
  }
  canvas {
    max-width: 100%;
    max-height: 100%;
    display: block;
  }
  .off {
    display: grid;
    place-items: center;
    color: var(--text-2);
  }
  .kind {
    position: absolute;
    left: 2px;
    bottom: 2px;
    display: grid;
    place-items: center;
    width: 15px;
    height: 15px;
    border-radius: 4px;
    background: rgb(0 0 0 / 0.65);
    color: #fff;
  }
  .text {
    min-width: 0;
    display: grid;
    gap: 1px;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    font-size: var(--fs-s);
    color: var(--text-0);
  }
  li.hidden .name {
    color: var(--text-2);
  }
  .state {
    font-size: var(--fs-xs);
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .state.warn {
    color: #e0a060;
  }
  .rename {
    position: absolute;
    left: 58px;
    right: 62px;
    top: 50%;
    transform: translateY(-50%);
    font: inherit;
    font-size: var(--fs-s);
    color: var(--text-0);
    background: var(--bg-0);
    border: 1px solid var(--line-strong);
    border-radius: var(--radius-s);
    padding: 2px 4px;
  }
  .actions {
    display: flex;
    padding-right: 2px;
    opacity: 0;
  }
  li:hover .actions,
  li:focus-within .actions,
  li.selected .actions,
  li.hidden .actions {
    opacity: 1;
  }
  footer {
    flex: none;
    padding: var(--space-2) var(--space-3);
    border-top: 1px solid var(--line);
    margin-top: var(--space-1);
  }
  label {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-s);
    color: var(--text-1);
    cursor: pointer;
  }
  input[type="checkbox"] {
    accent-color: #e5484d;
  }
  kbd {
    margin-left: auto;
    font-family: inherit;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
</style>
