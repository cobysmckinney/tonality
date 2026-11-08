<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import type { HistoryView } from "$lib/bindings/HistoryView";
  import { session } from "$lib/edit/session.svelte";
  import { layoutGraph, relativeTime } from "./graph";

  interface Props {
    view: HistoryView;
    onhide?: () => void;
  }
  let { view, onhide }: Props = $props();

  const ROW = 30;
  const LANE = 14;
  const PAD = 12;

  const g = $derived(layoutGraph(view));
  const gutter = $derived(PAD * 2 + (g.lanes - 1) * LANE);
  const x = (lane: number) => PAD + lane * LANE;
  const y = (row: number) => row * ROW + ROW / 2;

  function edgePath(e: (typeof g.edges)[number]): string {
    const [x1, y1, x2, y2] = [x(e.fromLane), y(e.fromRow), x(e.toLane), y(e.toRow)];
    if (x1 === x2) return `M${x1},${y1} V${y2}`;
    // Run down the branch's own lane, then bend into the parent on the row above it.
    const bend = y2 - ROW;
    return `M${x1},${y1} V${bend} C${x1},${bend + ROW * 0.6} ${x2},${y2 - ROW * 0.4} ${x2},${y2}`;
  }

  // Re-render relative times every 30s.
  let now = $state(Date.now());
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(t);
  });

  // ---- branch rename ----
  let renaming = $state<string | null>(null);
  let draft = $state("");

  function startRename(name: string) {
    renaming = name;
    draft = name;
  }
  async function finishRename() {
    const old = renaming;
    renaming = null;
    if (old === null || draft.trim() === old || !draft.trim()) return;
    await session.renameBranch(old, draft).catch(() => {});
  }
  function renameKey(e: KeyboardEvent) {
    e.stopPropagation();
    if (e.key === "Enter") (e.currentTarget as HTMLInputElement).blur();
    if (e.key === "Escape") {
      renaming = null;
    }
  }
  function focusSelect(el: HTMLInputElement) {
    el.focus();
    el.select();
  }

  async function branchFrom(commit: number) {
    await session.branch(commit);
    if (session.history) startRename(session.history.head.branch);
  }

  async function remove(name: string) {
    const ok = await ask(`Delete the branch “${name}”? Its edits will no longer be shown.`, {
      title: "Delete branch",
      kind: "warning",
      okLabel: "Delete",
    });
    if (ok) await session.deleteBranch(name).catch(() => {});
  }
</script>

<div class="panel">
  <header>
    <span class="title">History</span>
    <div class="tools">
      <button class="icon" onclick={() => branchFrom(view.head.commit)} title="New branch from here">
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
          ><circle cx="4" cy="3.5" r="1.6" /><circle cx="4" cy="12.5" r="1.6" /><circle cx="12" cy="5.5" r="1.6" /><path
            d="M4 5.1v5.8M12 7.1c0 3-8 2-8 3.8"
          /></svg
        >
      </button>
      {#if onhide}
        <button class="icon" onclick={onhide} title="Hide history">‹</button>
      {/if}
    </div>
  </header>

  <ul class="branches">
    {#each view.branches as b (b.name)}
      {@const current = b.name === view.head.branch}
      <li class:current>
        <span class="dot" style:background="var(--branch-{g.branchColor.get(b.name) ?? 0})"></span>
        {#if renaming === b.name}
          <input
            class="rename"
            bind:value={draft}
            use:focusSelect
            onblur={finishRename}
            onkeydown={renameKey}
            maxlength="64"
            spellcheck="false"
          />
        {:else}
          <button
            class="name"
            onclick={() => session.switchBranch(b.name)}
            ondblclick={() => startRename(b.name)}
            title="Switch to {b.name} · double-click to rename">{b.name}</button
          >
          {#if view.branches.length > 1}
            <button class="del" onclick={() => remove(b.name)} title="Delete branch">×</button>
          {/if}
        {/if}
      </li>
    {/each}
  </ul>

  <div class="graph" style:--gutter="{gutter}px">
    <svg width={gutter} height={g.rows.length * ROW} aria-hidden="true">
      {#each g.edges as e (e.fromRow)}
        <path d={edgePath(e)} stroke="var(--branch-{e.color})" class="edge" />
      {/each}
      {#each g.rows as r (r.commit.id)}
        <circle
          cx={x(r.lane)}
          cy={y(r.row)}
          r={r.isHead ? 5 : 3.5}
          class:head={r.isHead}
          class:ahead={r.ahead}
          style:--c="var(--branch-{r.color})"
        />
      {/each}
    </svg>

    <ol>
      {#each g.rows as r (r.commit.id)}
        <li class:head={r.isHead} class:ahead={r.ahead} style:height="{ROW}px">
          <button class="row" onclick={() => session.checkout(r.commit.id)}>
            <span class="summary">{r.commit.summary}</span>
            {#each r.tips as t (t.branch.name)}
              <span class="tag" style:--c="var(--branch-{t.color})">{t.branch.name}</span>
            {/each}
            <span class="time">{relativeTime(r.commit.time, now)}</span>
          </button>
          <button class="fork" onclick={() => branchFrom(r.commit.id)} title="New branch from this step">⑂</button>
        </li>
      {/each}
    </ol>
  </div>
</div>

<style>
  .panel {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
  }
  header {
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: var(--space-3) var(--space-3) var(--space-2) var(--space-4);
  }
  .title {
    font-weight: 500;
  }
  .tools {
    display: flex;
    gap: var(--space-1);
  }
  .icon {
    display: grid;
    place-items: center;
    width: 22px;
    height: 22px;
    border-radius: var(--radius-s);
    color: var(--text-2);
  }
  .icon:hover {
    color: var(--text-0);
    background: var(--bg-2);
  }
  .icon svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
  }

  /* ---- branches ---- */
  .branches {
    list-style: none;
    margin: 0;
    padding: 0 var(--space-2) var(--space-2);
    border-bottom: 1px solid var(--line);
  }
  .branches li {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 26px;
    padding: 0 var(--space-2);
    border-radius: var(--radius-s);
  }
  .branches li:hover {
    background: var(--bg-2);
  }
  .branches li.current {
    background: var(--bg-2);
  }
  .dot {
    flex: none;
    width: 8px;
    height: 8px;
    border-radius: 50%;
  }
  .name {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    text-align: left;
    color: var(--text-1);
    font-size: var(--fs-s);
  }
  .current .name {
    color: var(--text-0);
    font-weight: 500;
  }
  .rename {
    flex: 1;
    min-width: 0;
    font: inherit;
    font-size: var(--fs-s);
    color: var(--text-0);
    background: var(--bg-0);
    border: 1px solid var(--focus);
    border-radius: var(--radius-s);
    padding: 1px 4px;
    outline: none;
  }
  .del {
    visibility: hidden;
    color: var(--text-2);
    padding: 0 2px;
  }
  .branches li:hover .del {
    visibility: visible;
  }
  .del:hover {
    color: var(--danger);
  }

  /* ---- graph ---- */
  .graph {
    position: relative;
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    overflow-x: hidden;
    padding-bottom: var(--space-4);
  }
  svg {
    position: absolute;
    left: 0;
    top: 0;
    pointer-events: none;
  }
  .edge {
    fill: none;
    stroke-width: 1.5;
  }
  circle {
    fill: var(--bg-1);
    stroke: var(--c);
    stroke-width: 1.5;
  }
  circle.head {
    fill: var(--c);
    stroke: var(--text-0);
  }
  circle.ahead {
    opacity: 0.45;
  }
  ol {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  ol li {
    position: relative;
    display: flex;
    align-items: center;
  }
  .row {
    flex: 1;
    min-width: 0;
    height: 100%;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: 0 var(--space-3) 0 var(--gutter);
    text-align: left;
    color: var(--text-1);
    font-size: var(--fs-s);
  }
  ol li:hover {
    background: var(--bg-2);
  }
  li.head {
    background: color-mix(in srgb, var(--bg-3) 60%, transparent);
  }
  li.head .row {
    color: var(--text-0);
  }
  li.ahead .row {
    color: var(--text-2);
  }
  .summary {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tag {
    flex: none;
    max-width: 80px;
    overflow: hidden;
    text-overflow: ellipsis;
    font-size: var(--fs-xs);
    padding: 0 5px;
    border-radius: var(--radius-s);
    color: var(--c);
    border: 1px solid color-mix(in srgb, var(--c) 50%, transparent);
  }
  .time {
    flex: none;
    font-size: var(--fs-xs);
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .fork {
    position: absolute;
    right: var(--space-2);
    display: none;
    width: 20px;
    height: 20px;
    border-radius: var(--radius-s);
    background: var(--bg-3);
    color: var(--text-1);
    font-size: var(--fs-s);
  }
  ol li:hover .fork {
    display: grid;
    place-items: center;
  }
  ol li:hover .time {
    visibility: hidden;
  }
  .fork:hover {
    color: var(--text-0);
  }
</style>
