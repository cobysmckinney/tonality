<!--
  History, one version at a time.
  - Versions: every branch of this photo as a card (thumbnail, steps, what it was made from).
  - Steps: only the current version's edits, newest first. Steps it inherited from the version it
    was made from sit below a "From …" divider, so it's clear which work is whose.
-->
<script lang="ts">
  import { ask } from "@tauri-apps/plugin-dialog";
  import { api } from "$lib/api";
  import type { Change } from "$lib/bindings/Change";
  import type { CommitInfo } from "$lib/bindings/CommitInfo";
  import type { EditState } from "$lib/bindings/EditState";
  import type { HistoryView } from "$lib/bindings/HistoryView";
  import { session } from "$lib/edit/session.svelte";
  import { branchThumbs, requestBranchThumb } from "$lib/edit/versions";
  import { branchSteps, deletion, versions } from "./graph";
  import { sessionLabel, sessionLabels, stepTime } from "./sessions";
  import { DEFAULT_WIDTH, MAX_WIDTH, MIN_WIDTH, historyWidth } from "./width.svelte";
  import { branchThumbKey } from "$lib/edit/versions";

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

  // "Today" and "Yesterday" move on at midnight.
  let now = $state(Date.now());
  $effect(() => {
    const t = setInterval(() => (now = Date.now()), 60_000);
    return () => clearInterval(t);
  });

  // ---- sessions: one time label where a pause of over half an hour ends ----
  const sessionStart = $derived(sessionLabels(groups.flatMap((g) => g.steps.map((s) => s.commit))));

  // ---- hover a version: show it on the canvas, until the pointer leaves the list ----
  const PREVIEW_DELAY = 120;
  let hovered: string | null = null;
  let hoverTimer: ReturnType<typeof setTimeout> | undefined;
  let previewSeq = 0;
  /** Tip states, by commit and its last change (a tip can be updated in place). */
  const tipStates = new Map<string, EditState>();

  function hoverVersion(name: string | null) {
    if (name === hovered) return;
    hovered = name;
    clearTimeout(hoverTimer);
    if (name === null || name === current) {
      previewSeq++;
      session.preview = null;
      return;
    }
    hoverTimer = setTimeout(() => showPreview(name), PREVIEW_DELAY);
  }

  async function showPreview(name: string) {
    const seq = ++previewSeq;
    const tip = view.branches.find((b) => b.name === name)?.tip;
    if (tip === undefined) return;
    const key = `${tip}:${view.commits.find((c) => c.id === tip)?.time}`;
    let state = tipStates.get(key);
    if (!state) {
      state = await api.history.state(tip).catch(() => undefined);
      if (!state) return;
      tipStates.set(key, state);
    }
    if (seq === previewSeq && hovered === name) session.preview = { label: name, state };
  }

  function leaveVersions(e: FocusEvent | PointerEvent) {
    const to = e.relatedTarget as Node | null;
    if (!to || !(e.currentTarget as HTMLElement).contains(to)) hoverVersion(null);
  }

  // Never leave a preview behind (hiding the panel, leaving Edit).
  $effect(() => () => {
    clearTimeout(hoverTimer);
    session.preview = null;
  });

  // ---- step details: what a step changed, on hover or focus ----
  let tip = $state<{ commit: CommitInfo; top: number; bottom: number; left: number } | null>(null);
  let changes = $state<Change[] | null>(null);
  let tipTimer: ReturnType<typeof setTimeout> | undefined;
  let panel = $state<HTMLDivElement>();
  const changeCache = new Map<string, Change[]>();

  function showTip(el: HTMLElement, commit: CommitInfo, delay: number) {
    clearTimeout(tipTimer);
    tipTimer = setTimeout(async () => {
      const r = el.getBoundingClientRect();
      const right = panel?.getBoundingClientRect().right ?? r.right;
      tip = { commit, top: r.top, bottom: r.bottom, left: right + 6 };
      const key = `${commit.id}:${commit.time}`;
      changes = changeCache.get(key) ?? null;
      if (changes) return;
      const c = await api.history.changes(commit.id).catch(() => []);
      changeCache.set(key, c);
      if (tip?.commit.id === commit.id) changes = c;
    }, delay);
  }
  function hideTip() {
    clearTimeout(tipTimer);
    tip = null;
  }
  // Below the middle of the window, the details open upwards from the row.
  const tipUp = $derived(!!tip && tip.top > window.innerHeight / 2);

  // ---- resize from the right edge ----
  function startResize(e: PointerEvent) {
    if (e.button !== 0) return;
    e.preventDefault();
    const el = e.currentTarget as HTMLElement;
    el.setPointerCapture(e.pointerId);
    const x0 = e.clientX;
    const w0 = historyWidth.px;
    historyWidth.dragging = true;
    const move = (m: PointerEvent) => historyWidth.set(w0 + m.clientX - x0);
    const up = () => {
      historyWidth.dragging = false;
      historyWidth.save();
      el.removeEventListener("pointermove", move);
      el.removeEventListener("pointerup", up);
      el.removeEventListener("pointercancel", up);
    };
    el.addEventListener("pointermove", move);
    el.addEventListener("pointerup", up);
    el.addEventListener("pointercancel", up);
  }
  function resizeKey(e: KeyboardEvent) {
    const step = e.shiftKey ? 40 : 10;
    if (e.key === "ArrowLeft") historyWidth.set(historyWidth.px - step);
    else if (e.key === "ArrowRight") historyWidth.set(historyWidth.px + step);
    else if (e.key === "Home") historyWidth.set(MIN_WIDTH);
    else if (e.key === "End") historyWidth.set(MAX_WIDTH);
    else return;
    e.preventDefault();
    // The handle has used the key: it isn't also Edit's "previous / next photo".
    e.stopPropagation();
    historyWidth.save();
  }
  function resetWidth() {
    historyWidth.set(DEFAULT_WIDTH);
    historyWidth.save();
  }

  // ---- thumbnails (rendered with the live renderer, cached per commit) ----
  const thumbCommit = (name: string, tip: number) => (name === current ? view.head.commit : tip);
  /** When `commit` last changed: part of its thumbnail's key, since a step that later nudges
   *  are merged into keeps its id while its look changes. */
  const thumbTime = (commit: number) => view.commits.find((c) => c.id === commit)?.time ?? 0;
  $effect(() => {
    for (const v of all) {
      const commit = thumbCommit(v.name, v.tip);
      requestBranchThumb(view.imageId, commit, thumbTime(commit));
    }
  });
  const thumb = (name: string, tip: number) => {
    const commit = thumbCommit(name, tip);
    return branchThumbs.get(branchThumbKey(view.imageId, commit, thumbTime(commit))) ?? "";
  };

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

  const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? "" : "s"}`;

  async function remove(name: string) {
    const what = session.history && deletion(session.history, name);
    // Says what is lost, like the Reset dialog: the steps are deleted for good, not hidden.
    const lost = !what
      ? "Its steps are deleted."
      : what.steps === 0
        ? "It has no steps of its own, so no edits are lost."
        : `Its ${plural(what.steps, "step")} ${what.steps === 1 ? "is" : "are"} deleted.`;
    const kept = what?.steps ? " Steps it shares with other versions stay." : "";
    const moves = what?.switchesTo ? ` The photo switches to “${what.switchesTo}”.` : "";
    const ok = await ask(
      `Delete the version “${name}”? ${lost}${kept}${moves} This can't be undone.`,
      { title: "Delete version", kind: "warning", okLabel: "Delete", cancelLabel: "Cancel" },
    );
    if (ok) await session.deleteBranch(name).catch(() => {});
  }
</script>

<div class="panel" bind:this={panel}>
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
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path d="M8 3v10M3 8h10" />
          </svg>
        </button>
        {#if onhide}
          <button class="icon-btn" onclick={onhide} title="Hide panel">
            <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
              <path d="M10 3 5 8l5 5" />
            </svg>
          </button>
        {/if}
      </div>
    </header>

    <!-- Hovering (or focusing) a version shows it on the canvas until the pointer leaves the list. -->
    <ul onpointerleave={leaveVersions} onfocusout={leaveVersions}>
      {#each all as v (v.name)}
        {@const isCurrent = v.name === current}
        {@const src = thumb(v.name, v.tip)}
        <li
          class:current={isCurrent}
          style:--c="var(--branch-{v.color})"
          onpointerenter={() => hoverVersion(v.name)}
          onfocusin={() => hoverVersion(v.name)}
          onclickcapture={() => hoverVersion(null)}
        >
          <button
            class="card"
            onclick={() => !isCurrent && session.switchBranch(v.name)}
            ondblclick={() => startRename(v.name)}
            title={isCurrent
              ? "You're editing this version · double-click to rename"
              : `Switch to ${v.name}`}
          >
            <span class="thumb">
              {#if src}<img {src} alt="" />{/if}
            </span>
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
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <rect x="2" y="3" width="12" height="10" rx="1.5" /><path d="M8 3v10" />
                </svg>
              </button>
              <button
                class="icon-btn"
                onclick={() => (session.applying = v.name)}
                title="Merge from {v.name} into {current}…"
              >
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <circle cx="4.5" cy="3.5" r="1.5" /><circle cx="4.5" cy="12.5" r="1.5" /><circle
                    cx="11.5"
                    cy="8"
                    r="1.5"
                  /><path d="M4.5 5v6M10 8C7 8 4.5 7 4.5 5" />
                </svg>
              </button>
            {/if}
            <button class="icon-btn" onclick={() => startRename(v.name)} title="Rename">
              <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                <path d="M10.5 2.5l3 3L6 13H3v-3z" />
              </svg>
            </button>
            {#if all.length > 1}
              <button class="icon-btn danger" onclick={() => remove(v.name)} title="Delete version">
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <path d="M3 4.5h10M6.5 4.5V3h3v1.5M4.5 4.5l.6 8.5h5.8l.6-8.5" />
                </svg>
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
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path d="M6 4 3 7l3 3" /><path d="M3 7h6.5a3.5 3.5 0 0 1 0 7H8" />
          </svg>
        </button>
        <button
          class="icon-btn"
          onclick={session.redo}
          disabled={!view.canRedo}
          title="Redo (Ctrl+Shift+Z)"
        >
          <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
            <path d="m10 4 3 3-3 3" /><path d="M13 7H6.5a3.5 3.5 0 0 0 0 7H8" />
          </svg>
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
            {@const start = sessionStart.get(s.commit.id)}
            <li
              class:head={s.isHead}
              class:ahead={s.ahead}
              class:last={i === g.steps.length - 1}
              class:session-start={start !== undefined}
            >
              {#if start !== undefined}
                <span class="session">{sessionLabel(start, now)}</span>
              {/if}
              <button
                class="step"
                onclick={() => session.checkout(s.commit.id)}
                onpointerenter={(e) => showTip(e.currentTarget, s.commit, 350)}
                onpointerleave={hideTip}
                onfocus={(e) => showTip(e.currentTarget, s.commit, 0)}
                onblur={hideTip}
                aria-describedby={tip?.commit.id === s.commit.id ? "step-details" : undefined}
              >
                <span class="node"></span>
                <span class="summary">{s.commit.summary}</span>
              </button>
              <button
                class="fork icon-btn"
                onclick={() => branchFrom(s.commit.id)}
                title="New version from this step"
              >
                <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
                  <circle cx="4.5" cy="3.5" r="1.5" /><circle cx="4.5" cy="12.5" r="1.5" /><circle
                    cx="11.5"
                    cy="5.5"
                    r="1.5"
                  /><path d="M4.5 5v6M11.5 7c0 3-7 2-7 4" />
                </svg>
              </button>
              {#if s.forks.length}
                <div class="forks">
                  {#each s.forks as f (f)}
                    <button
                      class="chip"
                      style:--c="var(--branch-{colorOf.get(f) ?? 0})"
                      onclick={() => session.switchBranch(f)}
                      title="{f} was made from this step · switch to it"
                    >
                      ⑂ {f}
                    </button>
                  {/each}
                </div>
              {/if}
            </li>
          {/each}
        </ol>
      {/each}
    </div>
  </section>

  {#if tip}
    <div
      id="step-details"
      class="details"
      role="tooltip"
      style:left="{tip.left}px"
      style:top={tipUp ? undefined : `${tip.top}px`}
      style:bottom={tipUp ? `${window.innerHeight - tip.bottom}px` : undefined}
    >
      <div class="details-head">
        <span>{tip.commit.summary}</span>
        <span class="details-time">{stepTime(tip.commit.time, now)}</span>
      </div>
      {#if changes === null}
        <p class="details-note">…</p>
      {:else if changes.length === 0}
        <p class="details-note">
          {tip.commit.parent === null ? "The photo as shot." : "Nothing changed."}
        </p>
      {:else}
        <ul>
          {#each changes as c, k (k)}
            <li>
              <span class="details-label">{c.label}</span>
              {#if c.from != null && c.to != null}
                <span class="details-value">{c.from} → {c.to}</span>
              {/if}
            </li>
          {/each}
        </ul>
      {/if}
    </div>
  {/if}

  <!-- svelte-ignore a11y_no_noninteractive_tabindex, a11y_no_noninteractive_element_interactions -->
  <div
    class="resize"
    class:active={historyWidth.dragging}
    role="separator"
    aria-orientation="vertical"
    aria-label="Panel width"
    aria-valuenow={historyWidth.px}
    aria-valuemin={MIN_WIDTH}
    aria-valuemax={MAX_WIDTH}
    tabindex="0"
    title="Drag to resize · double-click for the default width"
    onpointerdown={startResize}
    onkeydown={resizeKey}
    ondblclick={resetWidth}
  ></div>
</div>

<style>
  .panel {
    position: relative;
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
  /* One time label per session, above its newest step. */
  .session {
    display: block;
    padding: var(--space-2) var(--space-2) 2px calc(var(--space-2) + 9px + var(--space-3));
    font-size: var(--fs-xs);
    color: var(--text-2);
    font-variant-numeric: tabular-nums;
  }
  li.session-start:not(:first-child) .node::before {
    display: none;
  }
  ol li.session-start .fork {
    top: auto;
    bottom: 3px;
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

  /* ---- step details ---- */
  .details {
    position: fixed;
    z-index: 50;
    width: max-content;
    min-width: 180px;
    max-width: 300px;
    max-height: 50vh;
    overflow: hidden;
    padding: var(--space-2) var(--space-3);
    border-radius: var(--radius-m);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-xs);
    white-space: normal;
    pointer-events: none;
  }
  .details-head {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    margin-bottom: var(--space-1);
    color: var(--text-0);
    font-size: var(--fs-s);
  }
  .details-time {
    flex: none;
    color: var(--text-2);
    font-size: var(--fs-xs);
    font-variant-numeric: tabular-nums;
  }
  .details ul {
    list-style: none;
    margin: 0;
    padding: 0;
    display: grid;
    gap: 2px;
  }
  .details li {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
  }
  .details-label {
    color: var(--text-1);
  }
  .details-value {
    flex: none;
    color: var(--text-0);
    font-family: var(--font-num);
  }
  .details-note {
    margin: 0;
    color: var(--text-2);
  }

  /* ---- resize ---- */
  .resize {
    position: absolute;
    top: 0;
    right: 0;
    bottom: 0;
    width: 6px;
    z-index: 3;
    cursor: col-resize;
  }
  .resize::after {
    content: "";
    position: absolute;
    top: 0;
    bottom: 0;
    right: 0;
    width: 2px;
    background: transparent;
    transition: background 120ms var(--ease);
  }
  .resize:hover::after,
  .resize:focus-visible::after,
  .resize.active::after {
    background: var(--line-strong);
  }
</style>
