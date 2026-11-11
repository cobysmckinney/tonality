<script lang="ts">
  // Virtualised thumbnail grid: only cells near the viewport are in the DOM.
  // With `grouped`, photos are laid out under a header per capture day.
  import { onMount, untrack } from "svelte";
  import { thumbKey, thumbnail } from "./thumbs";
  import type { ImageRow } from "$lib/bindings/ImageRow";
  import { dayKey, dayLabel } from "./dates";

  interface Props {
    images: ImageRow[];
    size: number;
    selected: Set<number>;
    focus: number | null;
    /** Section headers by capture day. */
    grouped?: boolean;
    scrollTop?: number;
    onselect: (id: number, e: MouseEvent) => void;
    /** Select every photo of one day (header click). */
    onselectday?: (ids: number[], e: MouseEvent) => void;
    onopen: (id: number) => void;
    onscroll?: (top: number) => void;
    /** The pointer moved far enough with a cell pressed to count as a drag. */
    ondragstart?: (id: number, x: number, y: number) => void;
  }
  let {
    images,
    size,
    selected,
    focus,
    grouped = false,
    scrollTop = 0,
    onselect,
    onselectday,
    onopen,
    onscroll,
    ondragstart,
  }: Props = $props();

  const GAP = 12;
  const PAD = 20;
  const LABEL = 30;
  const HEADER = 52;
  const OVERSCAN = 400; // px

  let el: HTMLDivElement;
  let width = $state(0);
  let height = $state(0);
  let scroll = $state(0);

  const cols = $derived(Math.max(1, Math.floor((width - 2 * PAD + GAP) / (size + GAP))));
  const cellW = $derived(Math.max(40, (width - 2 * PAD - GAP * (cols - 1)) / cols));
  const rowH = $derived(cellW + LABEL + GAP);

  interface Header {
    key: string;
    label: string;
    top: number;
    ids: number[];
  }
  interface Cell {
    img: ImageRow;
    top: number;
    left: number;
    /** Visual row, for keyboard navigation across sections. */
    row: number;
    col: number;
  }

  const layout = $derived.by(() => {
    const headers: Header[] = [];
    const cells: Cell[] = [];
    /** Indices into `cells`, per visual row. */
    const rows: number[][] = [];
    let y = PAD;
    let col = 0;
    let key: string | null = null;
    for (const img of images) {
      const k = grouped ? dayKey(img.meta?.capturedAt) : "";
      if (grouped && k !== key) {
        if (col > 0) y += rowH;
        if (headers.length) y += GAP;
        key = k;
        col = 0;
        headers.push({ key: k, label: dayLabel(k), top: y, ids: [] });
        y += HEADER;
      }
      if (col === 0) rows.push([]);
      headers.at(-1)?.ids.push(img.id);
      rows.at(-1)!.push(cells.length);
      cells.push({ img, top: y, left: PAD + col * (cellW + GAP), row: rows.length - 1, col });
      col++;
      if (col === cols) {
        col = 0;
        y += rowH;
      }
    }
    if (col > 0) y += rowH;
    return { headers, cells, rows, height: y + PAD, index: new Map(cells.map((c, i) => [c.img.id, i])) };
  });

  /** First cell whose bottom is below `y` (cells are in top order). */
  function firstBelow(y: number): number {
    const cs = layout.cells;
    let lo = 0;
    let hi = cs.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (cs[mid].top + rowH < y) lo = mid + 1;
      else hi = mid;
    }
    return lo;
  }

  const visible = $derived.by(() => {
    const top = scroll - OVERSCAN;
    const bottom = scroll + height + OVERSCAN;
    const out: Cell[] = [];
    for (let i = firstBelow(top); i < layout.cells.length && layout.cells[i].top < bottom; i++) out.push(layout.cells[i]);
    return out;
  });
  const visibleHeaders = $derived(
    layout.headers.filter((h) => h.top + HEADER > scroll - OVERSCAN && h.top < scroll + height + OVERSCAN),
  );

  /** The photo an arrow key moves focus to from `id`. */
  export function neighbor(id: number | null, key: string): number | null {
    const { cells, rows, index } = layout;
    if (!cells.length) return null;
    const i = id === null ? undefined : index.get(id);
    if (i === undefined) return cells[0].img.id;
    const c = cells[i];
    let j = i;
    if (key === "ArrowLeft") j = Math.max(0, i - 1);
    else if (key === "ArrowRight") j = Math.min(cells.length - 1, i + 1);
    else if (key === "ArrowUp" || key === "ArrowDown") {
      const r = rows[c.row + (key === "ArrowUp" ? -1 : 1)];
      if (r) j = r[Math.min(c.col, r.length - 1)];
    }
    return cells[j].img.id;
  }

  onMount(() => {
    el.scrollTop = scrollTop;
    scroll = el.scrollTop;
  });

  // Keep the focused cell in view during keyboard navigation: when focus moves, not whenever
  // the layout does (a rating change, a resize), which would yank the scroll back to it.
  $effect(() => {
    const f = focus;
    if (f === null) return;
    untrack(() => {
      const i = layout.index.get(f);
      if (i === undefined || !el) return;
      const head = grouped ? HEADER : 0;
      const top = layout.cells[i].top;
      const bottom = top + rowH;
      if (top < el.scrollTop + head) el.scrollTop = top - PAD - head;
      else if (bottom > el.scrollTop + el.clientHeight) el.scrollTop = bottom - el.clientHeight + PAD;
    });
  });

  function onScroll() {
    scroll = el.scrollTop;
    onscroll?.(scroll);
  }

  // ---- drag detection ----
  let press: { id: number; x: number; y: number } | null = null;
  let dragged = false;

  function onDown(e: PointerEvent, id: number) {
    if (e.button !== 0 || e.shiftKey || e.ctrlKey || e.metaKey) return;
    press = { id, x: e.clientX, y: e.clientY };
    dragged = false;
  }
  function onMove(e: PointerEvent) {
    if (!press || !ondragstart) return;
    if (Math.hypot(e.clientX - press.x, e.clientY - press.y) > 6) {
      dragged = true;
      ondragstart(press.id, e.clientX, e.clientY);
      press = null;
    }
  }
  function onClick(e: MouseEvent, id: number) {
    if (dragged) {
      dragged = false;
      return;
    }
    onselect(id, e);
  }
</script>

<svelte:window onpointerup={() => (press = null)} />

<!-- svelte-ignore a11y_no_static_element_interactions (pointer tracking for drags only) -->
<div
  class="scroller"
  bind:this={el}
  bind:clientWidth={width}
  bind:clientHeight={height}
  onscroll={onScroll}
  onpointermove={onMove}
>
  <div class="canvas" style:height="{layout.height}px">
    {#each visibleHeaders as h (h.key)}
      <div class="header" style:transform="translateY({h.top}px)" style:height="{HEADER}px">
        <button class="day" onclick={(e) => onselectday?.(h.ids, e)} title="Select this day's photos">
          {h.label}
        </button>
        <span class="n">{h.ids.length}</span>
      </div>
    {/each}
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
        onpointerdown={(e) => onDown(e, img.id)}
        onclick={(e) => onClick(e, img.id)}
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
  .header {
    position: absolute;
    top: 0;
    left: 0;
    right: 0;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 20px;
  }
  .day {
    font-size: var(--fs-l);
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text-0);
  }
  .day:hover {
    color: #fff;
    text-decoration: underline;
    text-decoration-color: var(--line-strong);
    text-underline-offset: 4px;
  }
  .n {
    font-size: var(--fs-xs);
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
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
    background: var(--bg-2);
    overflow: hidden;
    transition: background 100ms var(--ease);
  }
  .cell:hover .frame {
    background: var(--bg-3);
  }
  .cell.selected .frame {
    background: var(--bg-4);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .cell.focus .frame {
    box-shadow: inset 0 0 0 2px var(--focus);
  }
  img {
    max-width: calc(100% - 16px);
    max-height: calc(100% - 16px);
    object-fit: contain;
    border-radius: 3px;
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
