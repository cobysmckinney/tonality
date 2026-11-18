<script lang="ts">
  import { open } from "@tauri-apps/plugin-dialog";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import type { ExportColorSpace } from "$lib/bindings/ExportColorSpace";
  import type { ExportFormat } from "$lib/bindings/ExportFormat";
  import { exporter } from "./export.svelte";

  const s = $derived(exporter.settings);
  const count = $derived(exporter.ids?.length ?? 0);

  const formats: { id: ExportFormat; label: string; hint: string }[] = [
    { id: "jpeg", label: "JPEG", hint: "8-bit, for sharing" },
    { id: "png", label: "PNG", hint: "8-bit, lossless" },
    { id: "tiff", label: "TIFF", hint: "16-bit, for further editing" },
  ];
  const spaces: { id: ExportColorSpace; label: string; hint: string }[] = [
    { id: "srgb", label: "sRGB", hint: "Looks the same everywhere" },
    {
      id: "displayP3",
      label: "Display P3",
      hint: "Wider colour, for phones, Macs and wide-gamut screens",
    },
  ];
  const sizes: { label: string; value: number | null }[] = [
    { label: "Full", value: null },
    { label: "4096", value: 4096 },
    { label: "2048", value: 2048 },
    { label: "1080", value: 1080 },
  ];

  async function pickFolder() {
    const p = await open({
      directory: true,
      multiple: false,
      title: "Export to",
      defaultPath: s.folder || undefined,
    });
    if (typeof p === "string") exporter.settings.folder = p;
  }

  function onKey(e: KeyboardEvent) {
    if (!exporter.ids) return;
    // The dialog is modal: keep shortcuts from reaching Library/Edit underneath.
    e.stopPropagation();
    if (e.key === "Escape") {
      e.preventDefault();
      exporter.close();
    } else if (
      e.key === "Enter" &&
      !exporter.running &&
      !exporter.result &&
      !(e.target instanceof HTMLInputElement)
    ) {
      e.preventDefault();
      exporter.run();
    }
  }

  const pct = $derived(
    exporter.progress?.total ? (100 * exporter.progress.done) / exporter.progress.total : 0,
  );
  const folderName = (p: string) => p.split(/[\\/]/).filter(Boolean).pop() ?? p;
</script>

<svelte:window onkeydowncapture={onKey} />

{#if exporter.ids}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={() => exporter.close()}>
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-label="Export"
      tabindex="-1"
      onclick={(e) => e.stopPropagation()}
    >
      <h2>Export {count === 1 ? "photo" : `${count} photos`}</h2>

      {#if exporter.result}
        {@const r = exporter.result}
        <p class="done">
          {r.written.length} file{r.written.length === 1 ? "" : "s"} written to
          <strong>{folderName(r.folder)}</strong>
        </p>
        {#if r.failed.length}
          <ul class="failed">
            {#each r.failed as f (f)}<li>{f}</li>{/each}
          </ul>
        {/if}
        <div class="actions">
          {#if r.written.length}
            <button class="ghost" onclick={() => revealItemInDir(r.written[0])}
              >Show in Folder</button
            >
          {/if}
          <button class="primary" onclick={() => exporter.close()}>Done</button>
        </div>
      {:else}
        <div class="row">
          <span class="label">Versions</span>
          <div class="seg">
            <button
              class:active={s.versions === "current"}
              onclick={() => (exporter.settings.versions = "current")}>Current</button
            >
            <button
              class:active={s.versions === "all"}
              onclick={() => (exporter.settings.versions = "all")}>All branches</button
            >
          </div>
        </div>
        <p class="hint">
          {s.versions === "current"
            ? "The version on screen for each photo."
            : "One file per branch, named like “DSC0123 — B&W.jpg”."}
        </p>

        <div class="row">
          <span class="label">Format</span>
          <div class="seg">
            {#each formats as f (f.id)}
              <button
                class:active={s.format === f.id}
                onclick={() => (exporter.settings.format = f.id)}
                title={f.hint}>{f.label}</button
              >
            {/each}
          </div>
        </div>

        <div class="row">
          <span class="label">Colour</span>
          <div class="seg">
            {#each spaces as c (c.id)}
              <button
                class:active={s.colorSpace === c.id}
                onclick={() => (exporter.settings.colorSpace = c.id)}
                title={c.hint}>{c.label}</button
              >
            {/each}
          </div>
        </div>

        {#if s.format === "jpeg"}
          <div class="row">
            <span class="label">HDR</span>
            <div class="seg">
              <button class:active={!s.hdr} onclick={() => (exporter.settings.hdr = false)}
                >Off</button
              >
              <button class:active={s.hdr} onclick={() => (exporter.settings.hdr = true)}
                >Ultra HDR</button
              >
            </div>
          </div>
          {#if s.hdr}
            <p class="hint">
              HDR screens show highlights up to 10× brighter (1,000 nits); others show the photo as
              it is here.
            </p>
          {/if}
          <div class="row">
            <span class="label">Quality</span>
            <input
              type="range"
              min="50"
              max="100"
              value={s.quality}
              oninput={(e) => (exporter.settings.quality = Number(e.currentTarget.value))}
            />
            <span class="num">{s.quality}</span>
          </div>
        {/if}

        <div class="row">
          <span class="label">Long edge</span>
          <div class="seg">
            {#each sizes as z (z.label)}
              <button
                class:active={s.longEdge === z.value}
                onclick={() => (exporter.settings.longEdge = z.value)}>{z.label}</button
              >
            {/each}
          </div>
        </div>

        <div class="row">
          <span class="label">Folder</span>
          <button class="folder" onclick={pickFolder} title={s.folder}
            >{s.folder ? folderName(s.folder) : "Choose…"}</button
          >
        </div>

        {#if exporter.running}
          <div class="progress">
            <div class="bar"><div style:width="{pct}%"></div></div>
            <span>{exporter.progress?.name ?? "Starting…"}</span>
          </div>
        {/if}
        {#if exporter.error}<p class="error">{exporter.error}</p>{/if}

        <div class="actions">
          <button class="ghost" onclick={() => exporter.close()} disabled={exporter.running}
            >Cancel</button
          >
          <button
            class="primary"
            onclick={() => exporter.run()}
            disabled={exporter.running || !s.folder}
          >
            {exporter.running ? "Exporting…" : "Export"}
          </button>
        </div>
      {/if}
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
  .row {
    display: grid;
    grid-template-columns: 84px 1fr auto;
    align-items: center;
    gap: var(--space-2);
  }
  .label {
    font-size: var(--fs-s);
    color: var(--text-1);
  }
  .seg {
    grid-column: span 2;
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
  input[type="range"] {
    accent-color: var(--text-1);
  }
  .num {
    font-family: var(--font-num);
    font-size: var(--fs-s);
    color: var(--text-1);
    min-width: 2.5em;
    text-align: right;
  }
  .folder {
    grid-column: span 2;
    text-align: left;
    padding: 4px 8px;
    border-radius: var(--radius-s);
    background: var(--bg-0);
    color: var(--text-0);
    font-size: var(--fs-s);
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  .folder:hover {
    background: var(--bg-2);
  }
  .progress {
    display: grid;
    gap: 4px;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .bar {
    height: 3px;
    border-radius: 2px;
    background: var(--bg-0);
    overflow: hidden;
  }
  .bar div {
    height: 100%;
    background: var(--text-1);
    transition: width 150ms var(--ease);
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
  .done {
    margin: 0;
    color: var(--text-1);
    font-size: var(--fs-s);
  }
  .failed,
  .error {
    margin: 0;
    color: var(--danger);
    font-size: var(--fs-xs);
  }
</style>
