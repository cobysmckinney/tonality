<script lang="ts">
  // Virtualised thumbnail grid: only cells near the viewport are in the DOM.
  // With `grouped`, photos are laid out under a header per capture day.
  import { onMount, untrack } from "svelte";
  import { thumbKey, thumbnail } from "./thumbs";
  import type { ImageRow } from "$lib/bindings/ImageRow";
  import { dayKey, dayLabel } from "./dates";
  import { library } from "./library.svelte";

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

  const GAP = 6;
  const PAD = 20;
  const HEADER = 52;
  const OVERSCAN = 400; // px
  /** Shape assumed until a photo's thumbnail has been made (3:2, most cameras). */
  const DEFAULT_ASPECT = 1.5;

  let el: HTMLDivElement;
  let width = $state(0);
  let height = $state(0);
  let scroll = $state(0);

  /** Rows aim for this height; each is then scaled to fill the width exactly. */
  const targetH = $derived(Math.round(size * 0.75));

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
    w: number;
    h: number;
    /** Visual row, for keyboard navigation across sections. */
    row: number;
  }

  // Panoramas and very tall crops are kept within reason, so one can't take over a row.
  const aspectOf = (img: ImageRow) => Math.min(3, Math.max(0.4, img.aspect ?? DEFAULT_ASPECT));

  // Justified rows: photos keep their own shape, and each row (but a section's last) is scaled
  // so its photos fill the width exactly.
  const layout = $derived.by(() => {
    const headers: Header[] = [];
    const cells: Cell[] = [];
    /** Indices into `cells`, per visual row. */
    const rows: number[][] = [];
    const avail = Math.max(1, width - 2 * PAD);
    let y = PAD;
    let pending: ImageRow[] = [];

    const flush = (last: boolean) => {
      if (!pending.length) return;
      const sum = pending.reduce((s, img) => s + aspectOf(img), 0);
      const gaps = GAP * (pending.length - 1);
      // A full row fits the width; a section's last row keeps the target height (unless even
      // that overflows, which only a lone panorama can).
      const h = last ? Math.min(targetH, (avail - gaps) / sum) : (avail - gaps) / sum;
      let x = PAD;
      rows.push([]);
      pending.forEach((img, k) => {
        // The last photo of a full row takes up any rounding, so the right edge is straight.
        const w = !last && k === pending.length - 1 ? PAD + avail - x : aspectOf(img) * h;
        rows.at(-1)!.push(cells.length);
        cells.push({ img, top: y, left: x, w, h, row: rows.length - 1 });
        x += w + GAP;
      });
      y += h + GAP;
      pending = [];
    };

    let key: string | null = null;
    let rowAspect = 0;
    for (const img of images) {
      const k = grouped ? dayKey(img.meta?.capturedAt) : "";
      if (grouped && k !== key) {
        flush(true);
        if (headers.length) y += GAP * 2;
        key = k;
        rowAspect = 0;
        headers.push({ key: k, label: dayLabel(k), top: y, ids: [] });
        y += HEADER;
      }
      headers.at(-1)?.ids.push(img.id);
      pending.push(img);
      rowAspect += aspectOf(img);
      // Full once the photos at the target height would be wider than the space.
      if (rowAspect * targetH + GAP * (pending.length - 1) >= avail) {
        flush(false);
        rowAspect = 0;
      }
    }
    flush(true);
    return { headers, cells, rows, height: y - GAP + PAD, index: new Map(cells.map((c, i) => [c.img.id, i])) };
  });

  /** First cell whose bottom is below `y` (cells are in top order). */
  function firstBelow(y: number): number {
    const cs = layout.cells;
    let lo = 0;
    let hi = cs.length;
    while (lo < hi) {
      const mid = (lo + hi) >> 1;
      if (cs[mid].top + cs[mid].h < y) lo = mid + 1;
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
      // The photo in the next row up or down nearest this one's centre.
      const r = rows[c.row + (key === "ArrowUp" ? -1 : 1)];
      const mid = c.left + c.w / 2;
      if (r) j = r.reduce((best, k) => (Math.abs(cells[k].left + cells[k].w / 2 - mid) < Math.abs(cells[best].left + cells[best].w / 2 - mid) ? k : best));
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
      const bottom = top + layout.cells[i].h;
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
    {#each visible as { img, top, left, w, h } (img.id)}
      <div
        class="cell"
        class:selected={selected.has(img.id)}
        class:focus={focus === img.id}
        class:rejected={img.flag < 0}
        style:transform="translate({left}px, {top}px)"
        style:width="{w}px"
        style:height="{h}px"
        title={img.name}
        role="button"
        tabindex="-1"
        onpointerdown={(e) => onDown(e, img.id)}
        onclick={(e) => onClick(e, img.id)}
        ondblclick={() => onopen(img.id)}
        onkeydown={() => {}}
      >
        <div class="frame">
          <img
            use:thumbnail={thumbKey(img.id, img.head)}
            alt=""
            decoding="async"
            draggable="false"
            onload={(e) => {
              const t = e.currentTarget as HTMLImageElement;
              library.learnAspect(img.id, t.naturalWidth / t.naturalHeight);
            }}
          />
          {#if img.flag > 0}
            <span class="flag pick" title="Pick">●</span>
          {:else if img.flag < 0}
            <span class="flag reject" title="Rejected">✕</span>
          {/if}
          {#if img.branches > 1}
            <span class="versions" title="{img.branches} versions">{img.branches}</span>
          {/if}
          <div class="label">
            <span class="name">{img.name}</span>
            {#if img.rating > 0}
              <span class="stars" aria-label="{img.rating} stars">{"★".repeat(img.rating)}</span>
            {/if}
          </div>
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
    border-radius: var(--radius-s);
    cursor: default;
    outline: none;
  }
  /* The photo fills its card edge to edge; selection and focus ring it from outside. */
  .frame {
    position: relative;
    width: 100%;
    height: 100%;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    overflow: hidden;
  }
  .cell.selected .frame {
    box-shadow:
      0 0 0 2px var(--bg-0),
      0 0 0 4px var(--text-1);
  }
  .cell.focus .frame {
    box-shadow:
      0 0 0 2px var(--bg-0),
      0 0 0 4px var(--focus);
  }
  img {
    display: block;
    width: 100%;
    height: 100%;
    /* Matches the card's shape once known; until then, fills it rather than letterboxing. */
    object-fit: cover;
    user-select: none;
    opacity: 0;
    transition:
      opacity 160ms var(--ease),
      filter 120ms var(--ease);
  }
  img:global(.loaded) {
    opacity: 1;
  }
  img:not([src]) {
    display: none;
  }
  .cell:hover img {
    filter: brightness(1.08);
  }
  .rejected img:global(.loaded) {
    opacity: 0.35;
  }
  .flag,
  .versions,
  .stars {
    text-shadow: 0 1px 3px rgb(0 0 0 / 0.7);
  }
  .flag {
    position: absolute;
    top: 6px;
    left: 8px;
    font-size: 10px;
  }
  .pick {
    color: #fff;
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
    background: rgb(0 0 0 / 0.55);
    color: #fff;
    font-size: var(--fs-xs);
    text-align: center;
    line-height: 18px;
  }
  /* Name on hover, stars always: over the bottom of the photo, on a soft shade. */
  .label {
    position: absolute;
    left: 0;
    right: 0;
    bottom: 0;
    display: flex;
    align-items: flex-end;
    gap: var(--space-2);
    padding: 18px 8px 6px;
    font-size: var(--fs-xs);
    color: #fff;
    pointer-events: none;
    z-index: 1; /* its shade (::before) stays above the photo */
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    opacity: 0;
    transition: opacity 120ms var(--ease);
  }
  .label::before {
    content: "";
    position: absolute;
    inset: 0;
    z-index: -1;
    background: linear-gradient(transparent, rgb(0 0 0 / 0.55));
    opacity: 0;
    transition: opacity 120ms var(--ease);
  }
  .cell:hover .name,
  .cell:hover .label::before {
    opacity: 1;
  }
  .stars {
    flex: none;
    margin-left: auto;
    letter-spacing: 1px;
  }
</style>
