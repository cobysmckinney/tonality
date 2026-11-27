<!--
  Shown instead of the app when the library couldn't be opened at launch: it's already open in
  another tonality window, its folder can't be read, or it was last used by a newer version.
  Nothing else is mounted, because none of the commands the rest of the app calls can answer.
-->
<script lang="ts">
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api } from "$lib/api";
  import Logo from "$lib/design/Logo.svelte";

  let { message }: { message: string } = $props();

  async function showLog() {
    const path = await api.logPath().catch(() => null);
    if (path) await revealItemInDir(path).catch(() => {});
  }
</script>

<main>
  <div class="card" role="alert">
    <Logo kind="wordmark" height={18} label="tonality" />
    <h1>tonality can’t open your library</h1>
    <p class="why">{message}</p>
    <p class="note">Your photos and edits haven’t been changed.</p>
    <div class="actions">
      <button class="ghost" onclick={showLog}>Show Log File</button>
      <!-- svelte-ignore a11y_autofocus -->
      <button class="primary" autofocus onclick={() => api.quit()}>Quit</button>
    </div>
  </div>
</main>

<style>
  main {
    display: grid;
    place-items: center;
    height: 100%;
    padding: var(--space-5);
  }
  .card {
    display: grid;
    gap: var(--space-3);
    max-width: 520px;
    padding: var(--space-6);
    background: var(--bg-1);
    border: 1px solid var(--line);
    border-radius: var(--radius-l);
    color: var(--text-1);
  }
  h1 {
    margin: var(--space-3) 0 0;
    font-size: var(--fs-l);
    font-weight: 600;
    color: var(--text-0);
  }
  p {
    margin: 0;
    line-height: 1.5;
  }
  /* The one thing on screen worth copying into a bug report. */
  .why {
    color: var(--text-0);
    user-select: text;
    overflow-wrap: anywhere;
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-3);
  }
  /* The dialogs' buttons. */
  button {
    padding: 6px 14px;
    border-radius: var(--radius-s);
    font-size: var(--fs-m);
    font-weight: 500;
  }
  .primary {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .primary:hover {
    background: var(--line-strong);
  }
  .ghost {
    color: var(--text-1);
  }
  .ghost:hover {
    color: var(--text-0);
  }
</style>
