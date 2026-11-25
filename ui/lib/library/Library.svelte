<script lang="ts">
  import { onMount } from "svelte";
  import Logo from "$lib/design/Logo.svelte";
  import type { ImageMeta } from "$lib/bindings/ImageMeta";
  import type { ImageQuery } from "$lib/bindings/ImageQuery";
  import Grid from "./Grid.svelte";
  import { BIN as bin, library, type View } from "./state.svelte";
  import { thumbKey, thumbnail } from "./thumbs";
  import { targetOwnsKey } from "$lib/keys";
  import { palette } from "$lib/palette/commands.svelte";

  let grid = $state<ReturnType<typeof Grid> | null>(null);

  onMount(() => {
    library.refresh().then(() => library.recover());
  });

  const view = $derived(library.view);
  const inAlbum = $derived(view.kind === "album");
  const grouped = $derived(library.query.sort === "captureTime");
  const is = (v: View) =>
    v.kind === view.kind && (v.kind === "all" || ("id" in view && view.id === v.id));

  function onKey(e: KeyboardEvent) {
    // A focused control keeps the keys it uses (Enter on a button, anything in a field).
    if (targetOwnsKey(e.target, e.key)) return;
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
    const del = e.key === "Delete" || e.key === "Backspace";
    // Delete alone means "out of this album" inside one; with Ctrl it's always the library.
    if (del && !e.altKey && (mod || !inAlbum)) {
      e.preventDefault();
      library.remove();
      return;
    }
    if (mod || e.altKey) return;
    if (e.key.startsWith("Arrow")) {
      e.preventDefault();
      library.moveTo(grid?.neighbor(library.focus, e.key) ?? null, e.shiftKey);
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
    } else if (del) {
      library.removeFromAlbum();
    } else if (e.key === "Escape") {
      if (menuOpen) menuOpen = false;
      else library.clearSelection();
    }
  }

  function importDate(ms: number): string {
    const d = new Date(ms);
    const sameYear = d.getFullYear() === new Date().getFullYear();
    return d.toLocaleDateString(undefined, {
      month: "short",
      day: "numeric",
      year: sameYear ? undefined : "numeric",
    });
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
  const selCount = $derived(library.selected.size);

  // ---- album naming (new album / rename), inline in the sidebar ----
  /** `"new"` while naming a new album, an album id while renaming one. */
  let naming = $state<"new" | number | null>(null);
  let draft = $state("");
  /** Photos the new album will start with. */
  let newWith = $state<number[]>([]);

  function newAlbum(withPhotos: number[] = []) {
    menuOpen = false;
    newWith = withPhotos;
    draft = "";
    naming = "new";
  }
  // The command palette's actions that live in this component's state.
  onMount(() =>
    palette.provide({
      "library.newAlbum": () => newAlbum(),
      "library.newAlbumWith": () => newAlbum(library.targets()),
    }),
  );
  function startRename(id: number, name: string) {
    draft = name;
    naming = id;
  }
  async function finishNaming() {
    const n = naming;
    naming = null;
    const name = draft.trim();
    if (n === null || !name) return;
    if (n === "new") {
      const id = await library.createAlbum(name, newWith);
      // An empty album: go to it, ready to be filled.
      if (id !== undefined && !newWith.length) library.show({ kind: "album", id });
    } else if (name !== library.albums.find((a) => a.id === n)?.name) {
      await library.renameAlbum(n, name);
    }
  }
  function nameKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur();
    if (e.key === "Escape") {
      draft = "";
      (e.currentTarget as HTMLInputElement).blur();
    }
  }
  function focusSelect(el: HTMLInputElement) {
    el.focus();
    el.select();
  }

  // ---- "Add to album" menu ----
  let menuOpen = $state(false);
  let menuRoot = $state<HTMLDivElement>();
  function onWindowDown(e: PointerEvent) {
    if (menuOpen && menuRoot && !menuRoot.contains(e.target as Node)) menuOpen = false;
  }

  let importsOpen = $state(false);
  $effect(() => {
    if (view.kind === "import") importsOpen = true;
  });
</script>

<svelte:window onkeydown={onKey} onpointerdown={onWindowDown} />

<div class="library" class:dropping={library.dropping} class:dragging={!!library.drag}>
  {#if library.dropping}
    <div class="drop">Drop to import into your library</div>
  {/if}

  <!-- ============================ sidebar ============================ -->
  <aside class="rail surface">
    <div class="import-row">
      <button
        class="btn primary"
        onclick={() => library.importFolder()}
        disabled={!!library.importing}
        title="Import a folder (Ctrl+Shift+I)"
      >
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <path d="M8 2v8M5 7l3 3 3-3M3 11v2h10v-2" />
        </svg>
        Import Folder…
      </button>
      <button
        class="btn"
        onclick={() => library.importFiles()}
        disabled={!!library.importing}
        title="Import photos (Ctrl+O)"
      >
        Files…
      </button>
    </div>

    {#if library.importing}
      <div class="progress">
        <div class="bar">
          <div
            style:width="{library.importing.total
              ? (100 * library.importing.done) / library.importing.total
              : 0}%"
          ></div>
        </div>
        <span class="import-status">
          {#if library.stoppingImport}
            Stopping…
          {:else}
            {library.importing.total
              ? `Copying ${library.importing.done} of ${library.importing.total}`
              : "Looking for photos…"}
            <button
              class="stop"
              onclick={() => library.stopImport()}
              title="Stop importing. Photos already copied stay in your library."
            >
              Stop
            </button>
          {/if}
        </span>
      </div>
    {:else if library.restoring?.total}
      <div class="progress">
        <div class="bar">
          <div style:width="{(100 * library.restoring.done) / library.restoring.total}%"></div>
        </div>
        <span>Restoring {library.restoring.done} of {library.restoring.total}</span>
      </div>
    {/if}

    <nav>
      <div class="group">
        <div class="eyebrow head">Library</div>
        <button
          class="item"
          class:active={is({ kind: "all" })}
          onclick={() => library.show({ kind: "all" })}
        >
          <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true">
            <rect x="2" y="2" width="5" height="5" rx="1.2" /><rect
              x="9"
              y="2"
              width="5"
              height="5"
              rx="1.2"
            /><rect x="2" y="9" width="5" height="5" rx="1.2" /><rect
              x="9"
              y="9"
              width="5"
              height="5"
              rx="1.2"
            />
          </svg>
          <span class="label">All Photos</span>
          <span class="count">{library.total}</span>
        </button>
      </div>

      <div class="group">
        <div class="eyebrow head">
          Albums
          <button class="icon-btn add" onclick={() => newAlbum()} title="New album">
            <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
              <path d="M8 3v10M3 8h10" />
            </svg>
          </button>
        </div>

        {#if naming === "new"}
          <div class="item naming">
            <span class="cover"></span>
            <input
              class="field"
              bind:value={draft}
              use:focusSelect
              onblur={finishNaming}
              onkeydown={nameKey}
              placeholder={newWith.length
                ? `Album for ${newWith.length} photo${newWith.length === 1 ? "" : "s"}`
                : "Album name"}
              maxlength="120"
              spellcheck="false"
            />
          </div>
        {/if}

        {#each library.albums as a (a.id)}
          {#if naming === a.id}
            <div class="item naming">
              <span class="cover">
                {#if a.cover !== null}<img
                    use:thumbnail={thumbKey(a.cover, a.coverHead)}
                    alt=""
                    draggable="false"
                  />{/if}
              </span>
              <input
                class="field"
                bind:value={draft}
                use:focusSelect
                onblur={finishNaming}
                onkeydown={nameKey}
                maxlength="120"
                spellcheck="false"
              />
            </div>
          {:else}
            <div class="album" data-album={a.id} class:over={library.drag?.over === a.id}>
              <button
                class="item"
                class:active={is({ kind: "album", id: a.id })}
                onclick={() => library.show({ kind: "album", id: a.id })}
                ondblclick={() => startRename(a.id, a.name)}
              >
                <span class="cover">
                  {#if a.cover !== null}<img
                      use:thumbnail={thumbKey(a.cover, a.coverHead)}
                      alt=""
                      draggable="false"
                    />{/if}
                </span>
                <span class="label">{a.name}</span>
                <span class="count">{a.count}</span>
              </button>
              <div class="album-actions">
                <button
                  class="icon-btn"
                  onclick={() => startRename(a.id, a.name)}
                  title="Rename album"
                >
                  <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                    <path d="M10.5 2.5l3 3L6 13H3v-3z" />
                  </svg>
                </button>
                <button
                  class="icon-btn danger"
                  onclick={() => library.deleteAlbum(a.id)}
                  title="Delete album (photos stay in the library)"
                >
                  <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
                    <path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5" />
                  </svg>
                </button>
              </div>
            </div>
          {/if}
        {/each}

        {#if !library.albums.length && naming !== "new"}
          <button class="hint-card" onclick={() => newAlbum()}>
            <strong>Make an album</strong>
            Group photos from any day or import. Drag photos onto it to add them.
          </button>
        {/if}
      </div>

      {#if library.imports.length}
        <div class="group">
          <button
            class="eyebrow head toggle"
            onclick={() => (importsOpen = !importsOpen)}
            aria-expanded={importsOpen}
          >
            <svg
              class="chev"
              class:open={importsOpen}
              viewBox="0 0 16 16"
              width="10"
              height="10"
              aria-hidden="true"
            >
              <path d="m6 4 4 4-4 4" />
            </svg>
            Recent Imports
          </button>
          {#if importsOpen}
            {#each library.imports as m (m.id)}
              <button
                class="item import"
                class:active={is({ kind: "import", id: m.id })}
                onclick={() => library.show({ kind: "import", id: m.id })}
                title="From {m.source}"
              >
                <span class="label">
                  {m.name}
                  <span class="when">{importDate(m.importedAt)}</span>
                </span>
                <span class="count">{m.count}</span>
              </button>
            {/each}
          {/if}
        </div>
      {/if}
    </nav>
  </aside>

  <!-- ============================ grid ============================ -->
  <section class="main surface">
    <div class="toolbar">
      <div class="title">
        <span class="name">{library.viewName}</span>
        <span class="dim">{library.images.length}</span>
      </div>

      <div class="actions">
        {#if selCount}
          <span class="sel">{selCount} selected</span>
          <div class="menu" bind:this={menuRoot}>
            <button class="btn" class:on={menuOpen} onclick={() => (menuOpen = !menuOpen)}>
              <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
                <rect x="2" y="4" width="12" height="9" rx="1.5" /><path
                  d="M5 2h6M8 6.5v4M6 8.5h4"
                />
              </svg>
              Add to Album
            </button>
            {#if menuOpen}
              <div class="pop" role="menu">
                {#each library.albums.filter((a) => !is({ kind: "album", id: a.id })) as a (a.id)}
                  <button
                    role="menuitem"
                    onclick={() => {
                      menuOpen = false;
                      library.addToAlbum(a.id);
                    }}
                  >
                    <span class="cover">
                      {#if a.cover !== null}<img
                          use:thumbnail={thumbKey(a.cover, a.coverHead)}
                          alt=""
                          draggable="false"
                        />{/if}
                    </span>
                    <span class="label">{a.name}</span>
                    <span class="count">{a.count}</span>
                  </button>
                {/each}
                <button role="menuitem" class="new" onclick={() => newAlbum(library.targets())}>
                  <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                    <path d="M8 3v10M3 8h10" />
                  </svg>
                  New Album with {selCount === 1 ? "this photo" : `${selCount} photos`}…
                </button>
              </div>
            {/if}
          </div>
          <button
            class="btn"
            onclick={() => library.resetHistory()}
            title="Take the selected photos back to as-shot, deleting all their versions and steps"
          >
            Reset History…
          </button>
          <button
            class="btn"
            onclick={() => library.remove()}
            title="Remove from your library and move the files to the {bin} ({inAlbum
              ? 'Ctrl+Delete'
              : 'Delete'})"
          >
            Move to {bin}…
          </button>
          {#if inAlbum}
            <button
              class="btn"
              onclick={() => library.removeFromAlbum()}
              title="Remove from this album (Delete). The photos stay in your library."
            >
              Remove
            </button>
          {/if}
          <span class="sep"></span>
        {/if}

        <div class="stars" role="group" aria-label="Minimum rating">
          {#each [1, 2, 3, 4, 5] as n (n)}
            <button
              class:on={library.query.minRating >= n}
              onclick={() => library.setQuery({ minRating: library.query.minRating === n ? 0 : n })}
              title={library.query.minRating === n ? "Clear rating filter" : `${n}+ stars`}
            >
              ★
            </button>
          {/each}
        </div>

        <div class="seg" role="group" aria-label="Flag filter">
          {#each flagFilters as f (f.label)}
            <button
              class:active={library.query.flag === f.value}
              onclick={() => library.setQuery({ flag: f.value })}
              title={f.title}
            >
              {f.label}
            </button>
          {/each}
        </div>

        <div class="sort">
          <select
            value={library.query.sort}
            onchange={(e) =>
              library.setQuery({ sort: e.currentTarget.value as ImageQuery["sort"] })}
            aria-label="Sort"
          >
            <option value="captureTime">By date</option>
            <option value="importTime">By import</option>
            <option value="name">By name</option>
          </select>
          <button
            class="icon-btn"
            onclick={() => library.setQuery({ descending: !library.query.descending })}
            title={library.query.descending ? "Newest first" : "Oldest first"}
          >
            {library.query.descending ? "↓" : "↑"}
          </button>
        </div>

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
        {#key view.kind + ("id" in view ? view.id : "")}
          <Grid
            bind:this={grid}
            images={library.images}
            size={library.thumbSize}
            selected={library.selected}
            focus={library.focus}
            {grouped}
            scrollTop={library.scrollTop}
            onscroll={(t) => (library.scrollTop = t)}
            onselect={(id, e) => library.select(id, e)}
            onselectday={(ids, e) => {
              if (!(e.ctrlKey || e.metaKey || e.shiftKey)) library.selected.clear();
              for (const id of ids) library.selected.add(id);
              library.focus = library.anchor = ids[0];
            }}
            onopen={(id) => library.open(id)}
            ondragstart={(id, x, y) => library.startDrag(id, x, y)}
          />
        {/key}
      {:else if library.loaded}
        <div class="empty">
          {#if library.total === 0}
            <div class="mark"><Logo kind="mark" height={64} /></div>
            <p class="big">Your library is empty</p>
            <p class="dim">
              Import a folder or drop RAW files here. tonality keeps its own copy,<br />so the
              originals can stay wherever they are, or go.
            </p>
            <button
              class="btn primary"
              onclick={() => library.importFolder()}
              disabled={!!library.importing}
            >
              Import Folder…
            </button>
          {:else if inAlbum && library.query.minRating === 0 && library.query.flag === null}
            <p class="big">This album is empty</p>
            <p class="dim">
              Go to All Photos, select some, and drag them onto<br />“{library.viewName}” in the
              sidebar — or use Add to Album.
            </p>
            <button class="btn" onclick={() => library.show({ kind: "all" })}>
              Browse All Photos
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
      {:else}
        <span class="dim">
          Click to select · Enter to edit · 1–5 rate · P pick · X reject · Drag onto an album to add
        </span>
        {#if library.root}<span class="dim end" title="Your library folder">
            {library.root}
          </span>{/if}
      {/if}
    </footer>
  </section>
</div>

{#if library.drag}
  {@const d = library.drag}
  <div class="ghost" style:transform="translate({d.x + 14}px, {d.y + 14}px)">
    <span class="stack">
      <img
        use:thumbnail={thumbKey(
          d.ids[0],
          library.images.find((i) => i.id === d.ids[0])?.head ?? null,
        )}
        alt=""
      />
    </span>
    <span>
      {d.ids.length} photo{d.ids.length === 1 ? "" : "s"}
      <span class="to">
        {#if d.over !== null}→ {library.albums.find((a) => a.id === d.over)?.name}{:else}Drop on an
          album{/if}
      </span>
    </span>
  </div>
{/if}

<style>
  .library {
    position: relative;
    display: grid;
    grid-template-columns: 248px 1fr;
    gap: var(--gutter);
    height: 100%;
  }
  .library.dragging,
  .library.dragging :global(*) {
    cursor: grabbing !important;
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

  /* ---- sidebar ---- */
  .rail {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    padding: var(--space-3) var(--space-2);
    min-height: 0;
    overflow-y: auto;
  }
  .import-row {
    display: grid;
    grid-template-columns: 1fr auto;
    gap: var(--space-1);
    padding: 0 var(--space-1);
  }
  .progress {
    display: grid;
    gap: 6px;
    padding: 0 var(--space-2);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .import-status {
    display: flex;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--space-2);
  }
  .stop {
    color: var(--text-1);
    font-size: var(--fs-xs);
  }
  .stop:hover {
    color: var(--text-0);
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
    gap: var(--space-4);
  }
  .group {
    display: grid;
    gap: 2px;
  }
  .head {
    display: flex;
    align-items: center;
    justify-content: space-between;
    height: 26px;
    padding: 0 var(--space-1) 0 var(--space-2);
  }
  .head.toggle {
    justify-content: flex-start;
    gap: 6px;
  }
  .head.toggle:hover {
    color: var(--text-1);
  }
  .chev {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.8;
    transition: transform 150ms var(--ease);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .add {
    width: 22px;
    height: 22px;
  }
  .item {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    min-height: 34px;
    padding: 4px var(--space-2);
    border-radius: var(--radius-m);
    color: var(--text-1);
    text-align: left;
    transition: background 100ms var(--ease);
  }
  .item:hover {
    background: var(--bg-2);
  }
  .item.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .item svg {
    flex: none;
    margin: 0 4px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
  }
  .label {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    flex: none;
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .cover {
    flex: none;
    display: grid;
    place-items: center;
    width: 24px;
    height: 24px;
    border-radius: 5px;
    background: var(--bg-3);
    overflow: hidden;
  }
  .cover img {
    width: 100%;
    height: 100%;
    object-fit: cover;
    opacity: 0;
  }
  .cover img:global(.loaded) {
    opacity: 1;
  }
  .naming .field {
    flex: 1;
    min-width: 0;
  }
  .album {
    position: relative;
    border-radius: var(--radius-m);
  }
  .album.over {
    box-shadow: inset 0 0 0 1.5px var(--focus);
    background: color-mix(in srgb, var(--focus) 12%, transparent);
  }
  .album-actions {
    position: absolute;
    right: 4px;
    top: 50%;
    transform: translateY(-50%);
    /* Hidden but still focusable, so Tab reaches rename and delete. */
    display: flex;
    opacity: 0;
    pointer-events: none;
    gap: 1px;
    padding: 2px;
    border-radius: var(--radius-s);
    background: var(--bg-2);
  }
  .album:hover .album-actions,
  .album:focus-within .album-actions {
    opacity: 1;
    pointer-events: auto;
  }
  .dragging .album:hover .album-actions {
    opacity: 0;
    pointer-events: none;
  }
  .album-actions .icon-btn {
    width: 22px;
    height: 22px;
  }
  .danger:hover {
    color: var(--danger) !important;
  }
  .hint-card {
    display: grid;
    gap: 4px;
    margin: 2px var(--space-1) 0;
    padding: var(--space-3);
    border: 1px dashed var(--line-strong);
    border-radius: var(--radius-m);
    text-align: left;
    font-size: var(--fs-xs);
    line-height: 1.45;
    color: var(--text-2);
  }
  .hint-card strong {
    color: var(--text-1);
    font-weight: 500;
    font-size: var(--fs-s);
  }
  .hint-card:hover {
    border-color: var(--text-2);
  }
  .import {
    min-height: 40px;
  }
  .when {
    display: block;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }

  /* ---- main ---- */
  .main {
    display: grid;
    grid-template-rows: auto 1fr auto;
    min-width: 0;
    min-height: 0;
    container-type: inline-size;
  }
  /* Narrow windows: drop the least-used controls first. */
  @container (max-width: 900px) {
    input[type="range"] {
      display: none;
    }
  }
  @container (max-width: 760px) {
    .stars,
    .sel {
      display: none;
    }
  }
  @container (max-width: 700px) {
    .seg {
      display: none;
    }
  }
  .toolbar {
    display: flex;
    justify-content: space-between;
    align-items: center;
    gap: var(--space-4);
    height: 52px;
    padding: 0 var(--space-3) 0 20px;
    border-bottom: 1px solid var(--line);
  }
  .title {
    display: flex;
    align-items: baseline;
    gap: var(--space-2);
    flex: 1 1 auto;
    min-width: 90px;
    white-space: nowrap;
  }
  .title .name {
    font-size: var(--fs-l);
    font-weight: 600;
    letter-spacing: -0.01em;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .dim {
    color: var(--text-2);
    font-weight: 400;
  }
  .actions {
    flex: none;
    display: flex;
    align-items: center;
    gap: var(--space-3);
  }
  .sel {
    font-size: var(--fs-xs);
    color: var(--text-1);
    white-space: nowrap;
  }
  .sep {
    width: 1px;
    height: 20px;
    background: var(--line-strong);
  }
  .btn.on {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .menu {
    position: relative;
  }
  .pop {
    position: absolute;
    top: calc(100% + 6px);
    right: 0;
    z-index: 40;
    width: 260px;
    max-height: 60vh;
    overflow-y: auto;
    padding: var(--space-1);
    border-radius: var(--radius-m);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
  }
  .pop button {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    padding: 6px var(--space-2);
    border-radius: var(--radius-s);
    text-align: left;
    font-size: var(--fs-s);
    color: var(--text-1);
  }
  .pop button:hover {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .pop .new {
    margin-top: 2px;
    border-top: 1px solid var(--line);
    border-radius: 0 0 var(--radius-s) var(--radius-s);
    padding-top: 8px;
  }
  .pop .new:first-child {
    margin-top: 0;
    border-top: none;
    border-radius: var(--radius-s);
  }
  .pop svg {
    flex: none;
    margin: 0 6px;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.5;
  }
  .stars {
    display: flex;
  }
  .stars button {
    padding: 0 1px;
    color: var(--bg-4);
    font-size: 14px;
  }
  .stars button.on {
    color: var(--text-1);
  }
  .stars button:hover {
    color: var(--text-0);
  }
  .sort {
    display: flex;
    align-items: center;
    gap: 2px;
  }
  select {
    font: inherit;
    font-size: var(--fs-xs);
    color: var(--text-1);
    background: var(--bg-2);
    border: 1px solid var(--line);
    border-radius: var(--radius-s);
    height: 26px;
    padding: 0 6px;
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
    justify-items: center;
    gap: var(--space-2);
    text-align: center;
  }
  .empty p {
    margin: 0;
    line-height: 1.5;
  }
  .empty .mark {
    color: var(--text-2);
    margin-bottom: var(--space-3);
  }
  .empty .big {
    font-size: var(--fs-l);
    font-weight: 500;
  }
  .empty .btn {
    margin-top: var(--space-3);
    height: 34px;
    padding: 0 var(--space-5);
  }
  .info {
    display: flex;
    gap: var(--space-4);
    align-items: center;
    height: 34px;
    padding: 0 20px;
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

  /* ---- drag ghost ---- */
  .ghost {
    position: fixed;
    left: 0;
    top: 0;
    z-index: 100;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 6px 12px 6px 6px;
    border-radius: var(--radius-m);
    background: var(--bg-2);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-s);
    color: var(--text-0);
    pointer-events: none;
    white-space: nowrap;
  }
  .stack {
    display: grid;
    place-items: center;
    width: 36px;
    height: 36px;
    border-radius: 5px;
    background: var(--bg-0);
    overflow: hidden;
  }
  .stack img {
    max-width: 100%;
    max-height: 100%;
  }
  .to {
    display: block;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
</style>
