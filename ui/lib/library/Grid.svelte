<script lang="ts">
  // Virtualised thumbnail grid: only rows near the viewport are in the DOM.
  import { onMount } from "svelte";
  import { thumbKey, thumbnail } from "./thumbs";
  import type { ImageRow } from "$lib/bindings/ImageRow";

  interface Props {
    images: ImageRow[];
    size: number;
    selected: Set<number>;
    focus: number | null;
    /** Columns currently shown (for keyboard navigation). */
    cols?: number;
    scrollTop?: number;
    onselect: (id: number, e: MouseEvent) => void;
    onopen: (id: number) => void;
    onscroll?: (top: number) => void;
  }
  let {
    images,
    size,
    selected,
    focus,
    cols = $bindable(1),
    scrollTop = 0,
    onselect,
    onopen,
    onscroll,
  }: Props = $props();

  const GAP = 10;
  const PAD = 16;
  const LABEL = 30;
  const OVERSCAN = 2;

  let el: HTMLDivElement;
  let width = $state(0);
  let height = $state(0);
  let scroll = $state(0);

  $effect(() => {
    cols = Math.max(1, Math.floor((width - 2 * PAD + GAP) / (size + GAP)));
  });
  const cellW = $derived(Math.max(40, (width - 2 * PAD - GAP * (cols - 1)) / cols));
  const rowH = $derived(cellW + LABEL + GAP);
  const rows = $derived(Math.ceil(images.length / cols));
  const first = $derived(Math.max(0, Math.floor((scroll - PAD) / rowH) - OVERSCAN));
  const last = $derived(Math.min(rows, Math.ceil((scroll + height) / rowH) + OVERSCAN));
  const visible = $derived(
    images.slice(first * cols, last * cols).map((img, k) => {
      const i = first * cols + k;
      return { img, top: PAD + Math.floor(i / cols) * rowH, left: PAD + (i % cols) * (cellW + GAP) };
    }),
  );

  onMount(() => {
    el.scrollTop = scrollTop;
    scroll = el.scrollTop;
  });

  // Keep the focused cell in view during keyboard navigation.
  $effect(() => {
    const i = images.findIndex((x) => x.id === focus);
    if (i < 0 || !el) return;
    const top = PAD + Math.floor(i / cols) * rowH;
    const bottom = top + rowH;
    if (top < el.scrollTop) el.scrollTop = top - PAD;
    else if (bottom > el.scrollTop + el.clientHeight) el.scrollTop = bottom - el.clientHeight + PAD;
  });

  function onScroll() {
    scroll = el.scrollTop;
    onscroll?.(scroll);
  }
</script>

<div class="scroller" bind:this={el} bind:clientWidth={width} bind:clientHeight={height} onscroll={onScroll}>
  <div class="canvas" style:height="{rows * rowH + PAD * 2}px">
    {#each visible as { img, top, left } (img.id)}
      <div
        class="cell"
        class:selected={selected.has(img.id)}
        class:focus={focus === img.id}
        class:rejected={img.flag < 0}
        style:transform="translate({left}px, {top}px)"
        style:width="{cellW}px"
        role="button"
        tabindex="-1"
        onclick={(e) => onselect(img.id, e)}
        ondblclick={() => onopen(img.id)}
        onkeydown={() => {}}
      >
        <div class="frame" style:height="{cellW}px">
          <img use:thumbnail={thumbKey(img.id, img.head)} alt="" decoding="async" draggable="false" />
          {#if img.flag > 0}
            <span class="flag pick" title="Pick">●</span>
          {:else if img.flag < 0}
            <span class="flag reject" title="Rejected">✕</span>
          {/if}
          {#if img.branches > 1}
            <span class="versions" title="{img.branches} versions">{img.branches}</span>
          {/if}
        </div>
        <div class="label">
          <span class="name">{img.name}</span>
          {#if img.rating > 0}
            <span class="stars" aria-label="{img.rating} stars">{"★".repeat(img.rating)}</span>
          {/if}
        </div>
      </div>
    {/each}
  </div>
</div>

<style>
  .scroller {
    height: 100%;
    overflow-y: auto;
    overflow-x: hidden;
  }
  .canvas {
    position: relative;
  }
  .cell {
    position: absolute;
    top: 0;
    left: 0;
    border-radius: var(--radius-m);
    cursor: default;
    outline: none;
  }
  .frame {
    position: relative;
    display: grid;
    place-items: center;
    border-radius: var(--radius-m);
    background: var(--bg-1);
    overflow: hidden;
    transition: background 100ms var(--ease);
  }
  .cell:hover .frame {
    background: var(--bg-2);
  }
  .cell.selected .frame {
    background: var(--bg-3);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .cell.focus .frame {
    box-shadow: inset 0 0 0 2px var(--focus);
  }
  img {
    max-width: calc(100% - 16px);
    max-height: calc(100% - 16px);
    object-fit: contain;
    border-radius: 2px;
    user-select: none;
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
  .flag {
    position: absolute;
    top: 6px;
    left: 8px;
    font-size: 10px;
  }
  .pick {
    color: var(--text-0);
  }
  .reject {
    color: var(--danger);
  }
  .versions {
    position: absolute;
    top: 6px;
    right: 6px;
    min-width: 18px;
    padding: 0 5px;
    border-radius: 9px;
    background: color-mix(in srgb, var(--bg-0) 80%, transparent);
    color: var(--text-0);
    font-size: var(--fs-xs);
    text-align: center;
    line-height: 18px;
  }
  .label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 30px;
    padding: 0 4px;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .selected .label {
    color: var(--text-1);
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .stars {
    flex: none;
    color: var(--text-1);
    letter-spacing: 1px;
  }
</style>
