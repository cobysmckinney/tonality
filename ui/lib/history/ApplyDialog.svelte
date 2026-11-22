<script lang="ts">
  // "Copy settings from <branch>": pick which panels to take, applied as one commit.
  import type { Panel } from "$lib/bindings/Panel";
  import { session } from "$lib/edit/session.svelte";
  import { enterConfirms } from "$lib/keys";

  const panels: { id: Panel; label: string }[] = [
    { id: "whiteBalance", label: "White Balance" },
    { id: "light", label: "Light" },
    { id: "toneCurve", label: "Tone Curve" },
    { id: "color", label: "Color" },
    { id: "detail", label: "Detail" },
    { id: "effects", label: "Effects" },
    { id: "crop", label: "Crop" },
    { id: "masks", label: "Masks" },
    { id: "lens", label: "Lens Correction" },
  ];
  let chosen = $state<Set<Panel>>(new Set(["color"]));

  function toggle(p: Panel) {
    const next = new Set(chosen);
    if (next.has(p)) next.delete(p);
    else next.add(p);
    chosen = next;
  }

  async function apply() {
    const source = session.applying;
    if (!source || !chosen.size) return;
    session.applying = null;
    await session
      .applyFrom(
        source,
        panels.map((p) => p.id).filter((p) => chosen.has(p)),
      )
      .catch(() => {});
  }

  function onKey(e: KeyboardEvent) {
    if (!session.applying) return;
    e.stopPropagation(); // modal
    if (e.key === "Escape") session.applying = null;
    else if (enterConfirms(e)) {
      e.preventDefault();
      apply();
    }
  }
</script>

<svelte:window onkeydowncapture={onKey} />

{#if session.applying}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={() => (session.applying = null)}>
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-label="Copy settings"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      <h2>Copy settings from “{session.applying}”</h2>
      <p class="hint">Onto “{session.history?.head.branch}”, as one step you can undo.</p>
      <div class="grid">
        {#each panels as p (p.id)}
          <label>
            <input type="checkbox" checked={chosen.has(p.id)} onchange={() => toggle(p.id)} />
            {p.label}
          </label>
        {/each}
      </div>
      <div class="actions">
        <button class="link" onclick={() => (chosen = new Set(panels.map((p) => p.id)))}>
          All
        </button>
        <span class="spacer"></span>
        <button class="ghost" onclick={() => (session.applying = null)}>Cancel</button>
        <button class="primary" onclick={apply} disabled={!chosen.size}>Copy</button>
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
    place-items: center;
    background: rgba(0, 0, 0, 0.5);
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
    gap: var(--space-2);
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
