<!--
  Filmstrip: the Library's current view (All Photos, an album, an import, with its filters) as a
  strip under the canvas, so photos can be switched without leaving Edit.
  Click a photo, or ← / → for the previous / next one. Horizontally virtualised.
-->
<script lang="ts">
  import { library } from "$lib/library/library.svelte";
  import { thumbKey, thumbnail } from "$lib/library/thumbs";
  import { session } from "./session.svelte";

  const CELL = 76; // thumbnail box
  const GAP = 6;
  const STEP = CELL + GAP;
  const PAD = 10;
  const OVERSCAN = 6;

  let el = $state<HTMLDivElement>();
  let width = $state(0);
  let scroll = $state(0);

  $effect(() => {
    if (!library.loaded) library.refresh();
  });

  const images = $derived(library.images);
  const currentId = $derived(session.image?.id ?? null);
  const index = $derived(images.findIndex((i) => i.id === currentId));
  const first = $derived(Math.max(0, Math.floor((scroll - PAD) / STEP) - OVERSCAN));
  const last = $derived(Math.min(images.length, Math.ceil((scroll + width) / STEP) + OVERSCAN));

  // Keep the open photo in view (centred when it jumps off-screen).
  $effect(() => {
    if (index < 0 || !el) return;
    const left = PAD + index * STEP;
    if (left < el.scrollLeft || left + CELL > el.scrollLeft + el.clientWidth) {
      el.scrollTo({ left: left - (el.clientWidth - CELL) / 2, behavior: "smooth" });
    }
  });

  function open(id: number) {
    if (id === currentId) return;
    library.select(id);
    session.open(id);
  }

  // Vertical wheel scrolls the strip sideways.
  function onWheel(e: WheelEvent) {
    if (!el || Math.abs(e.deltaX) > Math.abs(e.deltaY)) return;
    e.preventDefault();
    el.scrollLeft += e.deltaY;
  }
</script>

<div class="strip">
  <div class="bar">
    <span class="source">{library.viewName}</span>
    {#if images.length}
      <span class="pos">{index >= 0 ? `${index + 1} of ${images.length}` : `${images.length} photos`}</span>
    {/if}
    <span class="keys">← → to switch photos</span>
  </div>
  <div
    class="scroller"
    bind:this={el}
    bind:clientWidth={width}
    onscroll={() => el && (scroll = el.scrollLeft)}
    onwheel={onWheel}
  >
    <div class="track" style:width="{PAD * 2 + images.length * STEP - GAP}px">
      {#each images.slice(first, last) as img, k (img.id)}
        {@const i = first + k}
        <button
          class="cell"
          class:current={img.id === currentId}
          class:rejected={img.flag < 0}
          style:transform="translateX({PAD + i * STEP}px)"
          style:width="{CELL}px"
          style:height="{CELL}px"
          onclick={() => open(img.id)}
          title={img.name}
        >
          <img use:thumbnail={thumbKey(img.id, img.head)} alt="" decoding="async" draggable="false" />
          {#if img.rating > 0}<span class="stars">{"★".repeat(img.rating)}</span>{/if}
          {#if img.flag > 0}<span class="flag">●</span>{/if}
        </button>
      {/each}
    </div>
    {#if library.loaded && !images.length}
      <p class="empty">No photos in {library.viewName}.</p>
    {/if}
  </div>
</div>

<style>
  .strip {
    display: grid;
    grid-template-rows: auto 1fr;
    min-width: 0;
    height: 100%;
  }
  .bar {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    padding: 8px var(--space-3) 0;
    font-size: var(--fs-xs);
    color: var(--text-2);
    white-space: nowrap;
  }
  .source {
    color: var(--text-1);
    font-weight: 500;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .pos {
    font-variant-numeric: tabular-nums;
  }
  .keys {
    margin-left: auto;
  }
  .scroller {
    position: relative;
    overflow-x: auto;
    overflow-y: hidden;
  }
  .scroller::-webkit-scrollbar {
    height: 6px;
  }
  .track {
    position: relative;
    height: 100%;
  }
  .cell {
    position: absolute;
    top: 6px;
    left: 0;
    display: grid;
    place-items: center;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    overflow: hidden;
    transition: background 100ms var(--ease);
  }
  .cell:hover {
    background: var(--bg-3);
  }
  .cell.current {
    background: var(--bg-4);
    box-shadow: inset 0 0 0 2px var(--text-0);
  }
  img {
    max-width: calc(100% - 8px);
    max-height: calc(100% - 8px);
    border-radius: 2px;
    opacity: 0;
    transition: opacity 160ms var(--ease);
  }
  img:global(.loaded) {
    opacity: 1;
  }
  img:not([src]) {
    display: none;
  }
  .rejected img:global(.loaded) {
    opacity: 0.35;
  }
  .stars,
  .flag {
    position: absolute;
    bottom: 3px;
    padding: 0 4px;
    border-radius: 4px;
    background: color-mix(in srgb, var(--bg-0) 75%, transparent);
    font-size: 9px;
    line-height: 14px;
    color: var(--text-0);
  }
  .stars {
    left: 3px;
    letter-spacing: 0.5px;
  }
  .flag {
    right: 3px;
  }
  .empty {
    margin: 0;
    padding: var(--space-4) var(--space-3);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
</style>
