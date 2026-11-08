<script lang="ts">
  // Phase 3: folder rail, virtualized grid, ratings/flags.
</script>

<div class="library">
  <aside class="rail">
    <div class="label">Folders</div>
    <p class="hint">No folders imported yet.</p>
  </aside>
  <section class="grid-area">
    <div class="empty">
      <p>Your library is empty.</p>
      <p class="hint">Import a folder of RAW files to get started.</p>
    </div>
  </section>
</div>

<style>
  .library {
    display: grid;
    grid-template-columns: 220px 1fr;
    height: 100%;
  }
  .rail {
    border-right: 1px solid var(--line);
    background: var(--bg-1);
    padding: var(--space-4);
  }
  .label {
    font-size: var(--fs-xs);
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-2);
    margin-bottom: var(--space-2);
  }
  .grid-area {
    display: grid;
    place-items: center;
  }
  .empty {
    text-align: center;
  }
  .hint {
    color: var(--text-2);
    font-size: var(--fs-s);
  }
</style>
