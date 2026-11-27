<script lang="ts">
  import { holdFocus } from "$lib/design/focus";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api } from "$lib/api";
  import type { ReleaseChannel } from "$lib/bindings/ReleaseChannel";
  import { updates } from "./updates.svelte";
  import { sections } from "$lib/design/sections.svelte";

  const channels: { id: ReleaseChannel; label: string; hint: string }[] = [
    { id: "stable", label: "Stable", hint: "Tested releases." },
    {
      id: "nightly",
      label: "Nightly",
      hint: "Built from the latest changes every night. Newer features, rougher edges.",
    },
  ];

  const s = $derived(updates.settings);
  const channel = $derived(channels.find((c) => c.id === s?.channel));
  const busy = $derived(
    updates.status === "checking" ||
      updates.status === "downloading" ||
      updates.status === "installing",
  );
  const pct = $derived(
    updates.progress?.total ? (100 * updates.progress.downloaded) / updates.progress.total : 0,
  );
  const mb = (bytes: number) => `${(bytes / 1_000_000).toFixed(1)} MB`;

  /** Show the log file in the file manager, for attaching to a bug report. */
  async function showLog() {
    const path = await api.logPath();
    if (path) await revealItemInDir(path);
  }

  function onKey(e: KeyboardEvent) {
    if (!updates.open) return;
    // The dialog is modal: keep shortcuts from reaching Library/Edit underneath.
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      updates.open = false;
    }
  }
</script>

<svelte:window onkeydowncapture={onKey} />

{#if updates.open && s}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={() => (updates.open = false)}>
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-label="Settings"
      tabindex="-1"
      use:holdFocus
      onclick={(e) => e.stopPropagation()}
    >
      <h2>Settings</h2>

      <h3>Updates</h3>
      <div class="row">
        <span class="label">Channel</span>
        <div class="seg">
          {#each channels as c (c.id)}
            <button
              class:active={s.channel === c.id}
              onclick={() => updates.setChannel(c.id)}
              disabled={updates.status === "installing"}
              title={c.hint}
            >
              {c.label}
            </button>
          {/each}
        </div>
      </div>
      <p class="hint">
        {channel?.hint}
        {#if s.channel === "stable" && updates.nightlyBuild}
          The latest stable release may be older than this nightly.
        {/if}
      </p>

      <div class="row">
        <span class="label">Automatic</span>
        <div class="seg">
          <button class:active={s.autoUpdate} onclick={() => updates.setAutoUpdate(true)}>
            On
          </button>
          <button class:active={!s.autoUpdate} onclick={() => updates.setAutoUpdate(false)}>
            Off
          </button>
        </div>
      </div>
      <p class="hint">
        {s.autoUpdate
          ? "Updates download in the background, then install when you restart."
          : "Check for updates yourself, below."}
      </p>

      <h3>Edit</h3>
      <div class="row">
        <span class="label">Sections</span>
        <div class="seg" role="radiogroup" aria-label="Open one section at a time">
          <button
            role="radio"
            aria-checked={!sections.oneAtATime}
            class:active={!sections.oneAtATime}
            onclick={() => sections.setOneAtATime(false)}
          >
            Any Number Open
          </button>
          <button
            role="radio"
            aria-checked={sections.oneAtATime}
            class:active={sections.oneAtATime}
            onclick={() => sections.setOneAtATime(true)}
          >
            One at a Time
          </button>
        </div>
      </div>
      <p class="hint">
        {sections.oneAtATime
          ? "Opening a section of the adjustments closes the others."
          : "Sections stay open until you close them."}
      </p>

      <div class="status">
        <div class="version">
          <span>tonality {updates.version}</span>
          {#if updates.status === "checking"}
            <span class="note">Checking for updates…</span>
          {:else if updates.status === "upToDate"}
            <span class="note">Up to date</span>
          {:else if updates.update && updates.status === "available"}
            <span class="note">{updates.update.version} is available</span>
          {:else if updates.update && updates.status === "downloading"}
            <span class="note">
              Downloading {updates.update.version}…
              {#if updates.progress}{mb(updates.progress.downloaded)}{/if}
            </span>
          {:else if updates.update && updates.status === "ready"}
            <span class="note">{updates.update.version} is ready to install</span>
          {:else if updates.status === "installing"}
            <span class="note">Installing…</span>
          {/if}
        </div>
        {#if updates.status === "downloading"}
          <div class="bar"><div style:width="{pct}%"></div></div>
        {/if}
        {#if updates.update?.notes && (updates.status === "available" || updates.status === "ready")}
          <pre class="notes">{updates.update.notes}</pre>
        {/if}
      </div>

      <div class="actions">
        <button
          class="ghost log"
          onclick={showLog}
          title="Errors are recorded in this file. Attach it when you report a problem."
        >
          Show Log File
        </button>
        {#if updates.status === "available"}
          <button class="primary" onclick={() => updates.download()}>Download</button>
        {:else if updates.status === "ready" || updates.status === "installing"}
          <button
            class="primary"
            onclick={() => updates.install()}
            disabled={updates.status === "installing"}
          >
            Restart to Update
          </button>
        {:else}
          <button class="ghost" onclick={() => updates.check()} disabled={busy}>
            Check for Updates
          </button>
        {/if}
        <button class="ghost" onclick={() => (updates.open = false)}>Done</button>
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
  /* Focus is put on the dialog itself when it opens; it needs no ring round all of it. */
  .dialog:focus {
    outline: none;
  }
  .dialog {
    width: min(420px, calc(100vw - 32px));
    padding: var(--space-5);
    border-radius: var(--radius-m);
    border: 1px solid var(--line);
    background: var(--bg-1);
    box-shadow: 0 20px 60px rgba(0, 0, 0, 0.5);
    display: grid;
    gap: var(--space-3);
  }
  h2 {
    margin: 0 0 var(--space-1);
    font-size: var(--fs-m, 15px);
    font-weight: 600;
  }
  h3 {
    margin: 0;
    font-size: var(--fs-xs);
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-2);
  }
  .row {
    display: grid;
    grid-template-columns: 84px 1fr;
    align-items: center;
    gap: var(--space-2);
  }
  .label {
    font-size: var(--fs-s);
    color: var(--text-1);
  }
  .seg {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--bg-0);
  }
  .seg button {
    flex: 1;
    padding: 4px 0;
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
    color: var(--text-2);
  }
  .seg button.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .hint {
    margin: calc(-1 * var(--space-2)) 0 0 calc(84px + var(--space-2));
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .status {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-3);
    border-radius: var(--radius-m);
    background: var(--bg-0);
  }
  .version {
    display: flex;
    justify-content: space-between;
    gap: var(--space-3);
    font-size: var(--fs-s);
    color: var(--text-0);
  }
  .version span:first-child {
    font-family: var(--font-num);
  }
  .note {
    color: var(--text-2);
    text-align: right;
  }
  .bar {
    height: 3px;
    border-radius: 2px;
    background: var(--bg-2);
    overflow: hidden;
  }
  .bar div {
    height: 100%;
    background: var(--text-1);
    transition: width 150ms var(--ease);
  }
  .notes {
    margin: 0;
    max-height: 160px;
    overflow: auto;
    white-space: pre-wrap;
    font: inherit;
    font-size: var(--fs-xs);
    color: var(--text-1);
  }
  .actions {
    display: flex;
    justify-content: flex-end;
    gap: var(--space-2);
    margin-top: var(--space-2);
  }
  .actions button {
    padding: 6px 16px;
    border-radius: var(--radius-m);
    font-weight: 500;
  }
  .actions .log {
    margin-right: auto;
    padding-left: 0;
    font-weight: 400;
  }
  .primary {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .primary:hover:not(:disabled) {
    background: var(--line-strong);
  }
  .ghost {
    color: var(--text-1);
  }
  .ghost:hover:not(:disabled) {
    color: var(--text-0);
  }
  button:disabled {
    opacity: 0.45;
  }
</style>
