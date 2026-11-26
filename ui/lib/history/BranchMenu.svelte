<script lang="ts">
  // Top-bar branch switcher: every version of the open photo, with a thumbnail.
  // Click to switch · compare side by side · copy settings from it.
  import { ask } from "@tauri-apps/plugin-dialog";
  import { session } from "$lib/edit/session.svelte";
  import { branchThumbs, requestBranchThumb } from "$lib/edit/versions";

  let open = $state(false);
  let root = $state<HTMLDivElement>();

  const view = $derived(session.history);
  const current = $derived(view?.head.branch ?? "");

  // Render thumbnails for every tip when the menu opens (cached per commit).
  $effect(() => {
    if (!open || !view) return;
    for (const b of view.branches)
      requestBranchThumb(view.imageId, b.name === current ? view.head.commit : b.tip);
  });

  function thumb(name: string, tip: number): string {
    if (!view) return "";
    const commit = name === current ? view.head.commit : tip;
    return branchThumbs.get(`${view.imageId}:${commit}`) ?? "";
  }

  function onWindowDown(e: PointerEvent) {
    if (open && root && !root.contains(e.target as Node)) open = false;
  }
  let trigger = $state<HTMLButtonElement>();
  function onKey(e: KeyboardEvent) {
    if (open && e.key === "Escape") {
      e.stopPropagation();
      // Focus goes back to the button it came from, not to nowhere: the row that had it is
      // removed with the list.
      const inside = root?.contains(document.activeElement);
      open = false;
      if (inside) trigger?.focus();
    }
  }

  async function newBranch() {
    if (!view) return;
    open = false;
    await session.branch(view.head.commit);
  }

  // Something to reset: more than the as-shot step, or more than one version.
  const resettable = $derived(!!view && (view.commits.length > 1 || view.branches.length > 1));
  const plural = (n: number, w: string) => `${n} ${w}${n === 1 ? "" : "s"}`;

  async function resetAll() {
    if (!view) return;
    open = false;
    const versions = plural(view.branches.length, "version");
    const steps = plural(view.commits.length - 1, "step");
    const ok = await ask(
      `Reset this photo to as-shot? All ${versions} and ${steps} of its history are deleted. This can't be undone.`,
      { title: "Reset history", kind: "warning", okLabel: "Reset", cancelLabel: "Cancel" },
    );
    if (ok) await session.resetHistory();
  }
</script>

<svelte:window onpointerdown={onWindowDown} onkeydowncapture={onKey} />

{#if view}
  <div class="menu" bind:this={root}>
    <button
      class="trigger"
      class:open
      bind:this={trigger}
      aria-expanded={open}
      aria-controls={open ? "versions-list" : undefined}
      onclick={() => (open = !open)}
      title="Versions of this photo"
    >
      <svg viewBox="0 0 16 16" width="13" height="13" aria-hidden="true">
        <circle cx="4" cy="3.5" r="1.6" /><circle cx="4" cy="12.5" r="1.6" /><circle
          cx="12"
          cy="5.5"
          r="1.6"
        /><path d="M4 5.1v5.8M12 7.1c0 3-8 2-8 3.8" />
      </svg>
      <span class="name">{current}</span>
      {#if view.branches.length > 1}<span class="count">{view.branches.length}</span>{/if}
      <span class="caret">▾</span>
    </button>

    {#if open}
      <!-- A group of buttons under a button that says it is open, not role="menu": each
           row holds several buttons, and a menu promises arrow keys and items only. -->
      <div class="pop" id="versions-list" role="group" aria-label="Versions of this photo">
        {#each view.branches as b (b.name)}
          {@const isCurrent = b.name === current}
          {@const src = thumb(b.name, b.tip)}
          <div class="item" class:current={isCurrent}>
            <button
              class="pick"
              aria-current={isCurrent ? "true" : undefined}
              onclick={() => {
                open = false;
                if (!isCurrent) session.switchBranch(b.name);
              }}
            >
              <span class="thumb">
                {#if src}<img {src} alt="" />{/if}
              </span>
              <span class="label">
                {b.name}{#if isCurrent}<em>editing</em>{/if}
              </span>
            </button>
            {#if !isCurrent}
              <div class="actions">
                <button
                  onclick={() => {
                    open = false;
                    session.startCompare(b.name);
                  }}
                  title="Compare side by side"
                  class:on={session.compare?.branch === b.name}
                >
                  Compare
                </button>
                <button
                  onclick={() => {
                    open = false;
                    session.applying = b.name;
                  }}
                  title="Merge some or all of its settings into {current}"
                >
                  Merge from {b.name}…
                </button>
              </div>
            {/if}
          </div>
        {/each}
        <button class="new" onclick={newBranch}>New version from current</button>
        <button
          class="reset"
          onclick={resetAll}
          disabled={!resettable}
          title="Delete every version and step, back to as-shot"
        >
          Reset all history…
        </button>
      </div>
    {/if}
  </div>
{/if}

<style>
  .menu {
    position: relative;
  }
  .trigger {
    display: flex;
    align-items: center;
    gap: 6px;
    max-width: 260px;
    height: 28px;
    padding: 0 10px;
    background: var(--bg-1);
    border-radius: var(--radius-m);
    color: var(--text-1);
  }
  .trigger:hover,
  .trigger.open {
    background: var(--bg-2);
    color: var(--text-0);
  }
  svg {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
  }
  .name {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .count {
    font-size: var(--fs-xs);
    color: var(--text-2);
    font-family: var(--font-num);
  }
  .caret {
    font-size: 9px;
    color: var(--text-2);
  }
  .pop {
    position: absolute;
    top: calc(100% + 6px);
    left: 0;
    z-index: 40;
    width: 300px;
    max-height: 70vh;
    overflow-y: auto;
    padding: var(--space-1);
    border-radius: var(--radius-l);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
  }
  .item {
    border-radius: var(--radius-m);
  }
  .item:hover,
  .item.current {
    background: var(--bg-2);
  }
  .pick {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    width: 100%;
    padding: var(--space-2);
    text-align: left;
  }
  .thumb {
    flex: none;
    display: grid;
    place-items: center;
    width: 56px;
    height: 56px;
    border-radius: var(--radius-s);
    background: var(--bg-0);
    overflow: hidden;
  }
  .thumb img {
    max-width: 100%;
    max-height: 100%;
  }
  .label {
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
    color: var(--text-0);
    font-size: var(--fs-s);
  }
  .label em {
    margin-left: 6px;
    font-style: normal;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .actions {
    display: flex;
    gap: var(--space-1);
    padding: 0 var(--space-2) var(--space-2) calc(56px + var(--space-2) + var(--space-3));
  }
  .actions button {
    padding: 1px 8px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--text-1);
    background: var(--bg-3);
  }
  .actions button:hover,
  .actions button.on {
    color: var(--text-0);
    background: var(--line-strong);
  }
  .new {
    display: block;
    width: 100%;
    margin-top: var(--space-1);
    padding: var(--space-2);
    border-top: 1px solid var(--line);
    text-align: left;
    font-size: var(--fs-s);
    color: var(--text-1);
  }
  .new:hover {
    color: var(--text-0);
  }
  .reset {
    display: block;
    width: 100%;
    padding: var(--space-2);
    text-align: left;
    font-size: var(--fs-s);
    color: var(--text-2);
  }
  .reset:hover:not(:disabled) {
    color: var(--danger);
  }
  .reset:disabled {
    opacity: 0.4;
  }
</style>
