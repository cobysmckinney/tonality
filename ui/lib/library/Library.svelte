<script lang="ts">
  import { onMount } from "svelte";
  import type { ImageMeta } from "$lib/bindings/ImageMeta";
  import type { ImageQuery } from "$lib/bindings/ImageQuery";
  import Grid from "./Grid.svelte";
  import { library } from "./library.svelte";

  let cols = $state(1);

  onMount(() => {
    library.refresh();
  });

  function onKey(e: KeyboardEvent) {
    if (e.target instanceof HTMLInputElement || e.target instanceof HTMLSelectElement) return;
    const mod = e.ctrlKey || e.metaKey;
    if (mod && e.key.toLowerCase() === "a") {
      e.preventDefault();
      library.selectAll();
      return;
    }
    if (mod && e.key.toLowerCase() === "i" && e.shiftKey) {
      e.preventDefault();
      library.importFolder();
      return;
    }
    if (mod && e.key.toLowerCase() === "o") {
      e.preventDefault();
      library.importFiles();
      return;
    }
    if (mod || e.altKey) return;
    const moves: Record<string, number> = { ArrowLeft: -1, ArrowRight: 1, ArrowUp: -cols, ArrowDown: cols };
    if (e.key in moves) {
      e.preventDefault();
      library.move(moves[e.key], e.shiftKey);
    } else if (/^[0-5]$/.test(e.key)) {
      library.setRating(Number(e.key));
    } else if (e.key === "p" || e.key === "P") {
      library.setFlag(1);
    } else if (e.key === "x" || e.key === "X") {
      library.setFlag(-1);
    } else if (e.key === "u" || e.key === "U") {
      library.setFlag(0);
    } else if (e.key === "Enter" && library.focus !== null) {
      e.preventDefault();
      library.open();
    } else if (e.key === "Escape") {
      library.clearSelection();
    }
  }

  const folderName = $derived(
    library.query.import === null
      ? "All Photos"
      : (library.imports.find((m) => m.id === library.query.import)?.name ?? "Import"),
  );

  function importDate(ms: number): string {
    const d = new Date(ms);
    const sameYear = d.getFullYear() === new Date().getFullYear();
    return d.toLocaleDateString(undefined, { month: "short", day: "numeric", year: sameYear ? undefined : "numeric" });
  }

  const flagFilters: { label: string; value: number | null; title: string }[] = [
    { label: "All", value: null, title: "Show all" },
    { label: "Picks", value: 1, title: "Picks only" },
    { label: "Unflagged", value: 0, title: "Unflagged only" },
    { label: "Rejected", value: -1, title: "Rejected only" },
  ];

  function shutter(t: number): string {
    if (t >= 0.3) return `${Number(t.toFixed(1))}s`;
    return `1/${Math.round(1 / t)}`;
  }

  function describe(m: ImageMeta | null): string[] {
    if (!m) return [];
    const out: string[] = [];
    if (m.focalLength) out.push(`${Math.round(m.focalLength)}mm`);
    if (m.fNumber) out.push(`ƒ/${Number(m.fNumber.toFixed(1))}`);
    if (m.exposureTime) out.push(shutter(m.exposureTime));
    if (m.iso) out.push(`ISO ${m.iso}`);
    return out;
  }

  function date(s: string | null | undefined): string {
    if (!s) return "";
    const d = new Date(s);
    return isNaN(d.getTime())
      ? s
      : d.toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
  }

  const focused = $derived(library.focused);
</script>

<svelte:window onkeydown={onKey} />

<div class="library" class:dropping={library.dropping}>
  {#if library.dropping}
    <div class="drop">Drop to import into your library</div>
  {/if}
  <aside class="rail">
    <div class="import-row">
      <button class="import" onclick={() => library.importFolder()} disabled={!!library.importing} title="Import a folder (Ctrl+Shift+I)">
        Import Folder…
      </button>
      <button class="import ghost" onclick={() => library.importFiles()} disabled={!!library.importing} title="Import photos (Ctrl+O)">
        Files…
      </button>
    </div>

    {#if library.importing}
      <div class="progress">
        <div class="bar">
          <div
            style:width="{library.importing.total ? (100 * library.importing.done) / library.importing.total : 0}%"
          ></div>
        </div>
        <span>
          {library.importing.total
            ? `Copying ${library.importing.done} of ${library.importing.total}`
            : "Looking for photos…"}
        </span>
      </div>
    {/if}

    <nav>
      <button
        class="folder"
        class:active={library.query.import === null}
        onclick={() => library.setQuery({ import: null })}
      >
        <span>All Photos</span><span class="count">{library.total}</span>
      </button>
      {#if library.imports.length}
        <div class="label">Imports</div>
      {/if}
      {#each library.imports as m (m.id)}
        <button
          class="folder"
          class:active={library.query.import === m.id}
          onclick={() => library.setQuery({ import: m.id })}
          title="From {m.source}"
        >
          <span class="fname">
            {m.name}
            <span class="when">{importDate(m.importedAt)}</span>
          </span>
          <span class="count">{m.count}</span>
        </button>
      {/each}
    </nav>

    {#if library.message}
      <p class="message">{library.message}</p>
    {/if}
  </aside>

  <section class="main">
    <div class="toolbar">
      <div class="title">
        <span>{folderName}</span>
        <span class="dim">{library.images.length}</span>
      </div>

      <div class="filters">
        <div class="stars" role="group" aria-label="Minimum rating">
          {#each [1, 2, 3, 4, 5] as n (n)}
            <button
              class:on={library.query.minRating >= n}
              onclick={() => library.setQuery({ minRating: library.query.minRating === n ? 0 : n })}
              title={library.query.minRating === n ? "Clear rating filter" : `${n}+ stars`}>★</button
            >
          {/each}
        </div>

        <div class="seg" role="group" aria-label="Flag filter">
          {#each flagFilters as f (f.label)}
            <button
              class:active={library.query.flag === f.value}
              onclick={() => library.setQuery({ flag: f.value })}
              title={f.title}>{f.label}</button
            >
          {/each}
        </div>

        <select
          value={library.query.sort}
          onchange={(e) => library.setQuery({ sort: e.currentTarget.value as ImageQuery["sort"] })}
          aria-label="Sort"
        >
          <option value="captureTime">Capture time</option>
          <option value="importTime">Import time</option>
          <option value="name">File name</option>
        </select>
        <button
          class="dir"
          onclick={() => library.setQuery({ descending: !library.query.descending })}
          title={library.query.descending ? "Newest first" : "Oldest first"}
        >
          {library.query.descending ? "↓" : "↑"}
        </button>

        <input
          type="range"
          min="100"
          max="360"
          step="10"
          value={library.thumbSize}
          oninput={(e) => library.setThumbSize(Number(e.currentTarget.value))}
          aria-label="Thumbnail size"
          title="Thumbnail size"
        />
      </div>
    </div>

    <div class="grid">
      {#if library.images.length}
        <Grid
          images={library.images}
          size={library.thumbSize}
          selected={library.selected}
          focus={library.focus}
          bind:cols
          scrollTop={library.scrollTop}
          onscroll={(t) => (library.scrollTop = t)}
          onselect={(id, e) => library.select(id, e)}
          onopen={(id) => library.open(id)}
        />
      {:else if library.loaded}
        <div class="empty">
          {#if library.total === 0}
            <p>Your library is empty.</p>
            <p class="dim">Import a folder or drop RAW files here. tonality keeps its own copy,</p>
            <p class="dim">so the originals can stay wherever they are, or go.</p>
            <button class="import big" onclick={() => library.importFolder()} disabled={!!library.importing}>
              Import Folder…
            </button>
          {:else}
            <p class="dim">No photos match these filters.</p>
          {/if}
        </div>
      {/if}
    </div>

    <footer class="info">
      {#if focused}
        <span class="strong">{focused.name}</span>
        {#if focused.meta}
          <span>{focused.meta.make} {focused.meta.model}</span>
          {#if focused.meta.lens}<span>{focused.meta.lens}</span>{/if}
          {#each describe(focused.meta) as d (d)}<span>{d}</span>{/each}
          <span class="dim">{date(focused.meta.capturedAt)}</span>
        {/if}
        {#if library.selected.size > 1}
          <span class="dim end">{library.selected.size} selected</span>
        {/if}
      {:else}
        <span class="dim">Click to select · Enter to edit · 1–5 rate · P pick · X reject · U unflag</span>
        {#if library.root}<span class="dim end" title="Your library folder">{library.root}</span>{/if}
      {/if}
    </footer>
  </section>
</div>

<style>
  .library {
    position: relative;
    display: grid;
    grid-template-columns: 220px 1fr;
    height: 100%;
  }
  .rail {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    border-right: 1px solid var(--line);
    background: var(--bg-1);
    padding: var(--space-4) var(--space-3);
    min-height: 0;
    overflow-y: auto;
  }
  .drop {
    position: absolute;
    inset: var(--space-3);
    z-index: 10;
    display: grid;
    place-items: center;
    border: 2px dashed var(--line-strong);
    border-radius: var(--radius-l);
    background: color-mix(in srgb, var(--bg-0) 85%, transparent);
    color: var(--text-0);
    font-size: var(--fs-l);
    pointer-events: none;
  }
  .import-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--space-1);
  }
  .import.ghost {
    background: var(--bg-2);
    color: var(--text-1);
    font-weight: 400;
  }
  .when {
    display: block;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .import {
    padding: 6px 12px;
    border-radius: var(--radius-m);
    background: var(--bg-3);
    color: var(--text-0);
    font-weight: 500;
  }
  .import:hover:not(:disabled) {
    background: var(--line-strong);
  }
  .import:disabled {
    opacity: 0.5;
  }
  .import.big {
    margin-top: var(--space-4);
    padding: 8px 18px;
  }
  .progress {
    display: grid;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .bar {
    height: 3px;
    border-radius: 2px;
    background: var(--bg-3);
    overflow: hidden;
  }
  .bar div {
    height: 100%;
    background: var(--text-1);
    transition: width 120ms linear;
  }
  nav {
    display: grid;
    gap: 1px;
  }
  .label {
    margin: var(--space-3) var(--space-2) var(--space-1);
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-2);
  }
  .folder {
    display: flex;
    justify-content: space-between;
    gap: var(--space-2);
    padding: 5px var(--space-2);
    border-radius: var(--radius-s);
    color: var(--text-1);
    text-align: left;
  }
  .folder:hover {
    background: var(--bg-2);
  }
  .folder.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .fname {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .message {
    margin: 0;
    padding: 0 var(--space-2);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }

  .main {
    display: grid;
    grid-template-rows: auto 1fr auto;
    min-width: 0;
    min-height: 0;
  }
  .toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--line);
    min-height: 40px;
  }
  .title {
    display: flex;
    gap: var(--space-2);
    font-weight: 500;
    white-space: nowrap;
  }
  .dim {
    color: var(--text-2);
    font-weight: 400;
  }
  .filters {
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .stars button {
    padding: 0 1px;
    color: var(--bg-3);
    font-size: 14px;
  }
  .stars button.on {
    color: var(--text-1);
  }
  .stars button:hover {
    color: var(--text-0);
  }
  .seg {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--bg-0);
  }
  .seg button {
    padding: 2px 8px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .seg button.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  select {
    font: inherit;
    font-size: var(--fs-xs);
    color: var(--text-1);
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: var(--radius-s);
    padding: 2px 4px;
  }
  .dir {
    color: var(--text-1);
    margin-left: calc(-1 * var(--space-2));
  }
  input[type="range"] {
    width: 90px;
    accent-color: var(--text-1);
  }
  .grid {
    min-height: 0;
    position: relative;
  }
  .empty {
    height: 100%;
    display: grid;
    place-content: center;
    text-align: center;
  }
  .empty p {
    margin: 2px;
  }
  .info {
    display: flex;
    gap: var(--space-4);
    align-items: center;
    height: 30px;
    padding: 0 var(--space-4);
    border-top: 1px solid var(--line);
    font-size: var(--fs-xs);
    color: var(--text-1);
    white-space: nowrap;
    overflow: hidden;
  }
  .strong {
    color: var(--text-0);
    font-weight: 500;
  }
  .end {
    margin-left: auto;
  }
</style>
