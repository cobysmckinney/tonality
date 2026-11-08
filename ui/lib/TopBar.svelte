<script lang="ts">
  import { app, type Mode } from "$lib/app.svelte";

  const modes: { id: Mode; label: string; key: string }[] = [
    { id: "library", label: "Library", key: "G" },
    { id: "edit", label: "Edit", key: "E" },
  ];
</script>

<header>
  <div class="brand">tonality</div>

  <nav class="modes" aria-label="Mode">
    {#each modes as m (m.id)}
      <button
        class:active={app.mode === m.id}
        onclick={() => (app.mode = m.id)}
        title="{m.label} ({m.key})"
      >
        {m.label}
      </button>
    {/each}
  </nav>

  <div class="right"></div>
</header>

<style>
  header {
    display: grid;
    grid-template-columns: 1fr auto 1fr;
    align-items: center;
    padding: 0 var(--space-4);
    border-bottom: 1px solid var(--line);
    background: var(--bg-1);
  }
  .brand {
    font-weight: 600;
    letter-spacing: -0.01em;
    color: var(--text-1);
  }
  .modes {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--bg-0);
  }
  .modes button {
    padding: 4px 14px;
    border-radius: var(--radius-s);
    color: var(--text-1);
    transition: background 120ms var(--ease), color 120ms var(--ease);
  }
  .modes button:hover {
    color: var(--text-0);
  }
  .modes button.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .right {
    justify-self: end;
  }
</style>
