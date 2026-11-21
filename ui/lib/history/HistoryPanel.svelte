<!--
  History, one version at a time.
  - Versions: every branch of this photo as a card (thumbnail, steps, what it was made from).
  - Steps: only the current version's edits, newest first. Steps it inherited from the version it
    was made from sit below a "From …" divider, so it's clear which work is whose.
-->
<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import type { HistoryView } from "$lib/bindings/HistoryView";
  import { session } from "$lib/edit/session.svelte";
  import { branchThumbs, requestBranchThumb } from "$lib/edit/versions";
  import { branchSteps, relativeTime, versions } from "./graph";

  interface Props {
    view: HistoryView;
    onhide?: () => void;
  }
  let { view, onhide }: Props = $props();

  const current = $derived(view.head.branch);
  const all = $derived(versions(view));
  const colorOf = $derived(new Map(all.map((v) => [v.name, v.color])));
  const groups = $derived(branchSteps(view));
  const ownSteps = $derived(groups[0]?.inherited ? 0 : (groups[0]?.steps.length ?? 0));

  // Re-render relative times every 30s.
  let now = $state(Date.now());
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 30_000);
    return () => clearInterval(t);
  });

  // ---- thumbnails (rendered with the live renderer, cached per commit) ----
  const thumbCommit = (name: string, tip: number) => (name === current ? view.head.commit : tip);
  $effect(() => {
    for (const v of all) requestBranchThumb(view.imageId, thumbCommit(v.name, v.tip));
  });
  const thumb = (name: string, tip: number) =>
    branchThumbs.get(`${view.imageId}:${thumbCommit(name, tip)}`) ?? "";

  // ---- rename ----
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
    if (e.key === "Escape") renaming = null;
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
    const ok = await ask(`Delete the version “${name}”? Its edits will no longer be shown.`, {
      title: "Delete version",
      kind: "warning",
      okLabel: "Delete",
    });
    if (ok) await session.deleteBranch(name).catch(() => {});
  }

  // Something to reset: more than the as-shot step, or more than one version.
  const resettable = $derived(view.commits.length > 1 || view.branches.length > 1);

  async function resetAll() {
    const versions = plural(view.branches.length, "version");
    const steps = plural(view.commits.length - 1, "step");
    const ok = await ask(
      `Reset this photo to as-shot? All ${versions} and ${steps} of its history are deleted. This can't be undone.`,
      { title: "Reset history", kind: "warning", okLabel: "Reset", cancelLabel: "Cancel" },
    );
    if (ok) await session.resetHistory();
  }

  const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? "" : "s"}`;
</script>

<div class="panel">
  <!-- ================= Versions ================= -->
  <section class="versions">
    <header>
      <span class="eyebrow">Versions</span>
      <div class="tools">
        <button
          class="icon-btn"
          onclick={() => branchFrom(view.head.commit)}
          title="New version from the current state"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
            ><path d="M8 3v10M3 8h10" /></svg
          >
        </button>
        {#if onhide}
          <button class="icon-btn" onclick={onhide} title="Hide panel">
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
              ><path d="M10 3 5 8l5 5" /></svg
            >
          </button>
        {/if}
      </div>
    </header>

    <ul>
      {#each all as v (v.name)}
        {@const isCurrent = v.name === current}
        {@const src = thumb(v.name, v.tip)}
        <li class:current={isCurrent} style:--c="var(--branch-{v.color})">
          <button
            class="card"
            onclick={() => !isCurrent && session.switchBranch(v.name)}
            ondblclick={() => startRename(v.name)}
            title={isCurrent
              ? "You're editing this version · double-click to rename"
              : `Switch to ${v.name}`}
          >
            <span class="thumb"
              >{#if src}<img {src} alt="" />{/if}</span
            >
            <span class="meta">
              {#if renaming === v.name}
                <input
                  class="field rename"
                  bind:value={draft}
                  use:focusSelect
                  onblur={finishRename}
                  onkeydown={renameKey}
                  onclick={(e) => e.stopPropagation()}
                  maxlength="64"
                  spellcheck="false"
                />
              {:else}
                <span class="name"><span class="dot"></span>{v.name}</span>
              {/if}
              <span class="sub">
                {#if isCurrent}<span class="badge">Editing</span>{/if}
                {v.parent ? `${plural(v.own, "step")} · from ${v.parent}` : plural(v.own, "step")}
              </span>
            </span>
          </button>
          <div class="actions">
            {#if !isCurrent}
              <button
                class="icon-btn"
                class:on={session.compare?.branch === v.name}
                onclick={() =>
                  session.compare?.branch === v.name
                    ? session.stopCompare()
                    : session.startCompare(v.name)}
                title="Compare side by side with {current}"
              >
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"
                  ><rect x="2" y="3" width="12" height="10" rx="1.5" /><path d="M8 3v10" /></svg
                >
              </button>
              <button
                class="icon-btn"
                onclick={() => (session.applying = v.name)}
                title="Copy settings from {v.name} onto {current}…"
              >
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"
                  ><rect x="5" y="5" width="8" height="8" rx="1.5" /><path
                    d="M11 5V3.5A1.5 1.5 0 0 0 9.5 2h-6A1.5 1.5 0 0 0 2 3.5v6A1.5 1.5 0 0 0 3.5 11H5"
                  /></svg
                >
              </button>
            {/if}
            <button class="icon-btn" onclick={() => startRename(v.name)} title="Rename">
              <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"
                ><path d="M10.5 2.5l3 3L6 13H3v-3z" /></svg
              >
            </button>
            {#if all.length > 1}
              <button class="icon-btn danger" onclick={() => remove(v.name)} title="Delete version">
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"
                  ><path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5" /></svg
                >
              </button>
            {/if}
          </div>
        </li>
      {/each}
    </ul>
  </section>

  <!-- ================= Steps of the current version ================= -->
  <section class="steps">
    <header>
      <span class="eyebrow">History <span class="of">· {current}</span></span>
      <div class="tools">
        <button
          class="icon-btn"
          onclick={session.undo}
          disabled={!view.canUndo}
          title="Undo (Ctrl+Z)"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
            ><path d="M6 4 3 7l3 3" /><path d="M3 7h6.5a3.5 3.5 0 0 1 0 7H8" /></svg
          >
        </button>
        <button
          class="icon-btn"
          onclick={session.redo}
          disabled={!view.canRedo}
          title="Redo (Ctrl+Shift+Z)"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true"
            ><path d="m10 4 3 3-3 3" /><path d="M13 7H6.5a3.5 3.5 0 0 0 0 7H8" /></svg
          >
        </button>
      </div>
    </header>

    <div class="list">
      {#if ownSteps === 0}
        <p class="empty">
          No edits on <strong>{current}</strong> yet. Changes you make now stay on this version.
        </p>
      {/if}

      {#each groups as g (g.branch)}
        {#if g.inherited}
          <div class="divider" style:--c="var(--branch-{colorOf.get(g.branch) ?? 0})">
            <span>From <strong>{g.branch}</strong></span>
          </div>
        {/if}
        <ol class:inherited={g.inherited} style:--c="var(--branch-{colorOf.get(g.branch) ?? 0})">
          {#each g.steps as s, i (s.commit.id)}
            <li class:head={s.isHead} class:ahead={s.ahead} class:last={i === g.steps.length - 1}>
              <button
                class="step"
                onclick={() => session.checkout(s.commit.id)}
                title="Go back to this step"
              >
                <span class="node"></span>
                <span class="summary">{s.commit.summary}</span>
                <span class="time">{relativeTime(s.commit.time, now)}</span>
              </button>
              <button
                class="fork icon-btn"
                onclick={() => branchFrom(s.commit.id)}
                title="New version from this step"
              >
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"
                  ><circle cx="4.5" cy="3.5" r="1.5" /><circle cx="4.5" cy="12.5" r="1.5" /><circle
                    cx="11.5"
                    cy="5.5"
                    r="1.5"
                  /><path d="M4.5 5v6M11.5 7c0 3-7 2-7 4" /></svg
                >
              </button>
              {#if s.forks.length}
                <div class="forks">
                  {#each s.forks as f (f)}
                    <button
                      class="chip"
                      style:--c="var(--branch-{colorOf.get(f) ?? 0})"
                      onclick={() => session.switchBranch(f)}
                      title="{f} was made from this step · switch to it">⑂ {f}</button
                    >
                  {/each}
                </div>
              {/if}
            </li>
          {/each}
        </ol>
      {/each}
    </div>
  </section>

  <footer>
    <button
      class="reset"
      onclick={resetAll}
      disabled={!resettable}
      title="Delete every version and step, back to as-shot"
    >
      <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true"
        ><path d="M3 3v4h4" /><path d="M3.5 7A5 5 0 1 1 5 11.5" /></svg
      >
      Reset all history…
    </button>
  </footer>
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
    height: 44px;
    padding: 0 var(--space-2) 0 var(--space-4);
  }
  .tools {
    display: flex;
    gap: 2px;
  }
  .of {
    text-transform: none;
    letter-spacing: 0;
    color: var(--text-1);
  }

  /* ---- versions ---- */
  .versions {
    flex: none;
    display: flex;
    flex-direction: column;
    max-height: 45%;
    border-bottom: 1px solid var(--line);
  }
  .versions ul {
    list-style: none;
    margin: 0;
    padding: 0 var(--space-2) var(--space-3);
    display: grid;
    gap: var(--space-1);
    overflow-y: auto;
  }
  .versions li {
    position: relative;
    border-radius: var(--radius-m);
    transition: background 120ms var(--ease);
  }
  .versions li:hover {
    background: var(--bg-2);
  }
  .versions li.current {
    background: var(--bg-2);
    box-shadow: inset 0 0 0 1px var(--line-strong);
  }
  .card {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-2);
    text-align: left;
  }
  .current .card {
    cursor: default;
  }
  .thumb {
    flex: none;
    display: grid;
    place-items: center;
    width: 48px;
    height: 48px;
    border-radius: var(--radius-s);
    background: var(--bg-0);
    overflow: hidden;
  }
  .thumb img {
    max-width: 100%;
    max-height: 100%;
  }
  .meta {
    display: grid;
    gap: 3px;
    min-width: 0;
    flex: 1;
  }
  .name {
    display: flex;
    align-items: center;
    gap: 6px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-1);
    font-size: var(--fs-s);
  }
  .current .name {
    color: var(--text-0);
    font-weight: 500;
  }
  .dot {
    flex: none;
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--c);
  }
  .sub {
    display: flex;
    align-items: center;
    gap: 6px;
    font-size: var(--fs-xs);
    color: var(--text-2);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .badge {
    padding: 0 6px;
    border-radius: var(--radius-pill);
    background: var(--bg-4);
    color: var(--text-0);
    line-height: 16px;
  }
  .rename {
    width: 100%;
    padding: 1px 6px;
  }
  .actions {
    position: absolute;
    right: var(--space-1);
    top: var(--space-1);
    /* Hidden but still focusable, so Tab reaches them. */
    display: flex;
    opacity: 0;
    pointer-events: none;
    gap: 1px;
    padding: 2px;
    border-radius: var(--radius-s);
    background: var(--bg-1);
    box-shadow: 0 0 0 1px var(--line);
  }
  .versions li:hover .actions,
  .versions li:focus-within .actions {
    opacity: 1;
    pointer-events: auto;
  }
  .actions .icon-btn {
    width: 22px;
    height: 22px;
  }
  .danger:hover {
    color: var(--danger) !important;
  }

  /* ---- steps ---- */
  footer {
    flex: none;
    padding: var(--space-2);
    border-top: 1px solid var(--line);
  }
  .reset {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    height: 30px;
    padding: 0 var(--space-2);
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .reset svg {
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .reset:hover:not(:disabled) {
    background: var(--bg-3);
    color: var(--danger);
  }
  .reset:disabled {
    opacity: 0.4;
  }
  .steps {
    flex: 1;
    min-height: 0;
    display: flex;
    flex-direction: column;
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: 0 var(--space-2) var(--space-4);
  }
  .empty {
    margin: 0 var(--space-2) var(--space-3);
    padding: var(--space-3);
    border-radius: var(--radius-m);
    border: 1px dashed var(--line-strong);
    font-size: var(--fs-xs);
    line-height: 1.5;
    color: var(--text-2);
  }
  .empty strong {
    color: var(--text-1);
    font-weight: 500;
  }
  .divider {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin: var(--space-3) var(--space-2) var(--space-1);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .divider::after {
    content: "";
    flex: 1;
    border-top: 1px dashed var(--line-strong);
  }
  .divider strong {
    color: var(--c);
    font-weight: 500;
  }
  ol {
    list-style: none;
    margin: 0;
    padding: 0;
  }
  ol li {
    position: relative;
    border-radius: var(--radius-s);
  }
  ol li:hover {
    background: var(--bg-2);
  }
  .step {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    height: 30px;
    padding: 0 var(--space-2);
    text-align: left;
    font-size: var(--fs-s);
    color: var(--text-1);
  }
  /* Timeline: a line through every node of a group. */
  .node {
    position: relative;
    flex: none;
    width: 9px;
    height: 9px;
    border-radius: 50%;
    border: 1.5px solid var(--c);
    background: var(--bg-1);
  }
  .node::before,
  .node::after {
    content: "";
    position: absolute;
    left: 2.25px;
    width: 1.5px;
    height: 11px;
    background: color-mix(in srgb, var(--c) 45%, transparent);
  }
  .node::before {
    bottom: 100%;
  }
  .node::after {
    top: 100%;
  }
  li:first-child .node::before,
  li.last .node::after {
    display: none;
  }
  li.head {
    background: var(--bg-3);
  }
  li.head .step {
    color: var(--text-0);
  }
  li.head .node {
    background: var(--c);
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--c) 25%, transparent);
  }
  li.ahead .step {
    color: var(--text-2);
  }
  li.ahead .node {
    border-style: dashed;
    opacity: 0.6;
  }
  .inherited .step {
    color: var(--text-2);
  }
  .inherited .node {
    opacity: 0.7;
  }
  .summary {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .time {
    flex: none;
    font-size: var(--fs-xs);
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  .fork {
    position: absolute;
    right: var(--space-1);
    top: 3px;
    /* Hidden but still focusable, so Tab reaches it. */
    display: inline-grid;
    opacity: 0;
    pointer-events: none;
    width: 24px;
    height: 24px;
    background: var(--bg-3);
  }
  ol li:hover .fork,
  ol li:focus-within .fork {
    opacity: 1;
    pointer-events: auto;
  }
  ol li:hover .time,
  ol li:focus-within .time {
    visibility: hidden;
  }
  .forks {
    display: flex;
    flex-wrap: wrap;
    gap: 4px;
    padding: 0 var(--space-2) var(--space-2) calc(var(--space-2) + 9px + var(--space-3));
  }
  .chip {
    padding: 0 8px;
    border-radius: var(--radius-pill);
    border: 1px solid color-mix(in srgb, var(--c) 45%, transparent);
    color: var(--c);
    font-size: var(--fs-xs);
    line-height: 18px;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .chip:hover {
    background: color-mix(in srgb, var(--c) 15%, transparent);
  }
</style>
