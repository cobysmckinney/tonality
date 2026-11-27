<script lang="ts">
  import { holdFocus } from "$lib/design/focus";
  // "Merge from <version>": pick which panels to take, applied as one commit. While it's open the
  // canvas shows the result for the panels ticked; Cancel puts the current edit back.
  import { api } from "$lib/api";
  import type { Panel } from "$lib/bindings/Panel";
  import { session } from "$lib/edit/session.svelte";
  import { enterConfirms } from "$lib/keys";
  import { MERGE_PANELS, orderedPanels, parsePanels } from "./merge";

  const KEY = "tonality.edit.mergePanels";
  let stored: string | null = null;
  try {
    stored = localStorage.getItem(KEY);
  } catch {
    /* a per-viewer convenience only */
  }
  let chosen = $state<Set<Panel>>(parsePanels(stored));

  function choose(next: Set<Panel>) {
    chosen = next;
    try {
      localStorage.setItem(KEY, JSON.stringify(orderedPanels(next)));
    } catch {
      /* ignore */
    }
  }

  function toggle(p: Panel) {
    const next = new Set(chosen);
    if (next.has(p)) next.delete(p);
    else next.add(p);
    choose(next);
  }

  // ---- live preview of the result ----
  let seq = 0;
  let previewing = false;
  $effect(() => {
    const source = session.applying;
    const id = session.history?.imageId;
    const panels = orderedPanels(chosen);
    void session.history?.head.commit; // the current edit moved: preview again
    const mine = ++seq; // a reply still on its way for an older choice is dropped
    if (!source || id === undefined || !panels.length) {
      // Closed, or nothing ticked: the current edit, unchanged.
      if (previewing) session.preview = null;
      previewing = false;
      return;
    }
    previewing = true;
    api.history
      .mergePreview(id, source, panels)
      .then((state) => {
        if (mine === seq && session.applying === source)
          session.preview = { label: `merged from ${source}`, state };
      })
      .catch(() => {});
  });

  function close() {
    session.applying = null;
  }

  async function apply() {
    const source = session.applying;
    if (!source || !chosen.size) return;
    close();
    await session.applyFrom(source, orderedPanels(chosen)).catch(() => {});
  }

  function onKey(e: KeyboardEvent) {
    if (!session.applying) return;
    e.stopPropagation(); // modal
    if (e.key === "Escape") close();
    else if (enterConfirms(e)) {
      e.preventDefault();
      apply();
    }
  }
</script>

<svelte:window onkeydowncapture={onKey} />

{#if session.applying}
  <!-- The scrim doesn't darken: the photo behind it is the preview. -->
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={close}>
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-label="Merge from {session.applying}"
      tabindex="-1"
      use:holdFocus
      onclick={(e) => e.stopPropagation()}
    >
      <h2>Merge from “{session.applying}”</h2>
      <p class="hint">
        Into “{session.history?.head.branch}”, as one step you can undo. The photo shows the result.
      </p>
      <div class="grid">
        {#each MERGE_PANELS as p (p.id)}
          <label>
            <input type="checkbox" checked={chosen.has(p.id)} onchange={() => toggle(p.id)} />
            {p.label}
          </label>
        {/each}
      </div>
      <div class="actions">
        <button class="link" onclick={() => choose(new Set(MERGE_PANELS.map((p) => p.id)))}>
          All
        </button>
        <button class="link" onclick={() => choose(new Set())}>None</button>
        <span class="spacer"></span>
        <button class="ghost" onclick={close}>Cancel</button>
        <button class="primary" onclick={apply} disabled={!chosen.size}>Merge</button>
      </div>
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 50;
    display: grid;
    place-items: end center;
    padding-bottom: 120px;
  }
  /* Focus is put on the dialog itself when it opens; it needs no ring round all of it. */
  .dialog:focus {
    outline: none;
  }
  .dialog {
    width: min(360px, calc(100vw - 32px));
    padding: var(--space-5);
    border-radius: var(--radius-m);
    border: 1px solid var(--line);
    background: var(--bg-1);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    display: grid;
    gap: var(--space-3);
  }
  h2 {
    margin: 0;
    font-size: var(--fs-m, 15px);
    font-weight: 600;
    overflow-wrap: anywhere;
  }
  .hint {
    margin: calc(-1 * var(--space-2)) 0 0;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .grid {
    display: grid;
    grid-template-columns: 1fr 1fr;
    gap: var(--space-2);
  }
  label {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    font-size: var(--fs-s);
    color: var(--text-1);
    cursor: pointer;
  }
  input {
    accent-color: var(--text-1);
  }
  .actions {
    display: flex;
    gap: var(--space-3);
    align-items: center;
    margin-top: var(--space-2);
  }
  .spacer {
    flex: 1;
  }
  .actions button {
    padding: 6px 16px;
    border-radius: var(--radius-m);
    font-weight: 500;
  }
  .link {
    padding: 0 !important;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .link:hover {
    color: var(--text-0);
  }
  .primary {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .primary:hover:not(:disabled) {
    background: var(--line-strong);
  }
  .primary:disabled {
    opacity: 0.45;
  }
  .ghost {
    color: var(--text-1);
  }
</style>
