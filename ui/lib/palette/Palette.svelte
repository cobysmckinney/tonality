<script lang="ts">
  import { tick } from "svelte";
  import { targetOwnsKey } from "$lib/keys";
  import { COMMANDS, MAC, palette, shortcut } from "./commands.svelte";
  import { available, keyLabels, search, shortcutGroups, type Command } from "./registry";

  const query = $derived(palette.query);
  let active = $state(0);
  let input = $state<HTMLInputElement>();
  let list = $state<HTMLElement>();

  type Row = { command: Command; hits: number[]; heading?: string };

  /** The commands view: ranked matches, or with no query, recently used then every group. */
  const rows = $derived.by((): Row[] => {
    if (palette.view !== "commands") return [];
    const ranked = search(available(COMMANDS), query, palette.recent);
    if (query.trim()) return ranked;
    let group = "";
    return ranked.map((r, i): Row => {
      const recent = palette.recent.includes(r.command.id);
      const heading = recent ? (i === 0 ? "Recently used" : undefined) : r.command.group;
      if (heading && heading !== group) {
        group = heading;
        return { ...r, heading };
      }
      return r;
    });
  });

  /** The shortcuts view: every shortcut, available now or not, filtered by the query. */
  const groups = $derived.by(() => {
    if (palette.view !== "shortcuts") return [];
    const withKeys = COMMANDS.filter((c) => c.keys?.length);
    const shown = query.trim() ? search(withKeys, query, []).map((r) => r.command) : withKeys;
    return shortcutGroups(shown);
  });

  $effect(() => {
    // A new query or view starts at the top.
    void query;
    void palette.view;
    active = 0;
  });

  $effect(() => {
    if (!palette.open) return;
    list?.querySelector(`[data-index="${active}"]`)?.scrollIntoView({ block: "nearest" });
  });

  function segments(title: string, hits: number[]) {
    const set = new Set(hits);
    const out: { text: string; hit: boolean }[] = [];
    for (let i = 0; i < title.length; i++) {
      const hit = set.has(i);
      const last = out[out.length - 1];
      if (last && last.hit === hit) last.text += title[i];
      else out.push({ text: title[i], hit });
    }
    return out;
  }

  async function run(c: Command) {
    palette.close();
    // Let the palette go (and focus go back) before the command opens anything of its own.
    await tick();
    palette.run(c);
  }

  function setView(v: "commands" | "shortcuts") {
    palette.view = v;
    input?.focus();
  }

  /** Another modal dialog is open (Settings, Export…): it keeps the keyboard. */
  const otherModal = () => !!document.querySelector("[aria-modal='true']:not([data-palette])");

  function onKey(e: KeyboardEvent) {
    if (e.isComposing) return;
    const mod = e.ctrlKey || e.metaKey;
    const k = !e.altKey && !e.shiftKey && mod && e.key.toLowerCase() === "k";
    if (!palette.open) {
      if (otherModal()) return;
      if (k) {
        e.preventDefault();
        e.stopPropagation();
        palette.show("commands");
      } else if (e.key === "?" && !mod && !e.altKey && !targetOwnsKey(e.target, e.key)) {
        e.preventDefault();
        e.stopPropagation();
        palette.show("shortcuts");
      }
      return;
    }
    // Modal: nothing reaches Library or Edit underneath.
    e.stopPropagation();
    if (e.key === "Escape" || k) {
      e.preventDefault();
      palette.close();
    } else if (e.key === "Tab") {
      trapTab(e);
    } else if (palette.view === "commands") {
      const n = rows.length;
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        if (n) active = (active + (e.key === "ArrowDown" ? 1 : n - 1)) % n;
      } else if (e.key === "PageDown" || e.key === "PageUp") {
        e.preventDefault();
        if (n) active = Math.max(0, Math.min(n - 1, active + (e.key === "PageDown" ? 8 : -8)));
      } else if (e.key === "Enter") {
        e.preventDefault();
        const r = rows[active];
        if (r) run(r.command);
      }
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      list?.scrollBy({ top: e.key === "ArrowDown" ? 48 : -48 });
    }
  }

  let dialog = $state<HTMLElement>();
  function trapTab(e: KeyboardEvent) {
    if (!dialog) return;
    const items = [...dialog.querySelectorAll<HTMLElement>("input, button:not([disabled])")];
    if (!items.length) return;
    e.preventDefault();
    const i = items.indexOf(document.activeElement as HTMLElement);
    const next = e.shiftKey ? (i <= 0 ? items.length - 1 : i - 1) : (i + 1) % items.length;
    items[next].focus();
  }

  /** On open: focus the field. On close: give focus back to what had it. */
  function holdFocus(node: HTMLElement) {
    const back = document.activeElement as HTMLElement | null;
    node.querySelector("input")?.focus();
    return {
      destroy() {
        if (back?.isConnected) back.focus();
      },
    };
  }
</script>

<svelte:window onkeydowncapture={onKey} />

{#snippet keys(c: Command)}
  <span class="keys">
    {#each c.keys ?? [] as combo, i (combo)}
      {#if i}<span class="or">or</span>{/if}
      <span class="combo">
        {#each keyLabels(combo, MAC) as label, j (j)}<kbd>{label}</kbd>{/each}
      </span>
    {/each}
  </span>
{/snippet}

{#if palette.open}
  <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
  <div class="scrim" onclick={() => palette.close()}>
    <!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
    <div
      class="dialog"
      role="dialog"
      aria-modal="true"
      aria-label={palette.view === "commands" ? "Commands" : "Keyboard shortcuts"}
      data-palette
      tabindex="-1"
      bind:this={dialog}
      use:holdFocus
      onclick={(e) => e.stopPropagation()}
    >
      <div class="head">
        <svg viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <circle cx="7" cy="7" r="4.5" /><path d="m10.5 10.5 3 3" />
        </svg>
        <input
          bind:this={input}
          bind:value={palette.query}
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-list"
          aria-autocomplete="list"
          aria-activedescendant={palette.view === "commands" && rows.length
            ? `palette-row-${active}`
            : undefined}
          placeholder={palette.view === "commands" ? "Search commands" : "Search shortcuts"}
          spellcheck="false"
          autocomplete="off"
        />
        <div class="seg" role="group" aria-label="View">
          <button
            class:active={palette.view === "commands"}
            onclick={() => setView("commands")}
            title="Commands ({shortcut('Mod+K')})"
          >
            Commands
          </button>
          <button
            class:active={palette.view === "shortcuts"}
            onclick={() => setView("shortcuts")}
            title="Keyboard shortcuts (?)"
          >
            Shortcuts
          </button>
        </div>
      </div>

      {#if palette.view === "commands"}
        <div class="list" id="palette-list" role="listbox" bind:this={list} aria-label="Commands">
          {#each rows as r, i (r.command.id)}
            {#if r.heading}<div class="heading" role="presentation">{r.heading}</div>{/if}
            <div
              id="palette-row-{i}"
              class="row"
              class:active={i === active}
              role="option"
              aria-selected={i === active}
              tabindex="-1"
              data-index={i}
              onpointermove={() => (active = i)}
              onclick={() => run(r.command)}
            >
              <span class="title">
                {#each segments(r.command.title, r.hits) as s, j (j)}
                  {#if s.hit}<mark>{s.text}</mark>{:else}{s.text}{/if}
                {/each}
              </span>
              {#if query.trim()}<span class="group">{r.command.group}</span>{/if}
              {@render keys(r.command)}
            </div>
          {:else}
            <p class="none">No command matches “{query.trim()}”.</p>
          {/each}
        </div>
      {:else}
        <div class="list sheet" id="palette-list" bind:this={list}>
          {#each groups as g (g.group)}
            <section>
              <h3>{g.group}</h3>
              {#each g.items as c (c.id)}
                <div class="shortcut">
                  <span class="title">{c.title}</span>
                  {@render keys(c)}
                </div>
              {/each}
            </section>
          {:else}
            <p class="none">No shortcut matches “{query.trim()}”.</p>
          {/each}
        </div>
      {/if}

      <div class="foot">
        {#if palette.view === "commands"}
          <span><kbd>↑</kbd><kbd>↓</kbd> move</span>
          <span><kbd>{MAC ? "↩" : "Enter"}</kbd> run</span>
        {:else}
          <span>Crop and Masks keys work while those tools are open.</span>
        {/if}
        <span class="end"><kbd>Esc</kbd> close</span>
      </div>
    </div>
  </div>
{/if}

<style>
  .scrim {
    position: fixed;
    inset: 0;
    z-index: 60;
    display: flex;
    justify-content: center;
    align-items: flex-start;
    padding-top: 12vh;
    background: rgba(0, 0, 0, 0.45);
  }
  .dialog {
    width: min(580px, calc(100vw - 32px));
    max-height: 72vh;
    display: flex;
    flex-direction: column;
    border-radius: var(--radius-l);
    background: var(--bg-1);
    box-shadow: var(--shadow-pop);
    overflow: hidden;
  }
  .dialog:has(.sheet) {
    width: min(760px, calc(100vw - 32px));
  }
  .head {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-4);
    border-bottom: 1px solid var(--line);
  }
  .head svg {
    flex: none;
    fill: none;
    stroke: var(--text-2);
    stroke-width: 1.5;
  }
  input {
    flex: 1;
    min-width: 0;
    height: 32px;
    border: none;
    outline: none;
    background: none;
    color: var(--text-0);
    font: inherit;
    font-size: var(--fs-l);
  }
  input::placeholder {
    color: var(--text-2);
  }
  .seg {
    display: flex;
    gap: 2px;
    padding: 2px;
    border-radius: var(--radius-m);
    background: var(--bg-0);
  }
  .seg button {
    padding: 3px 10px;
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
    color: var(--text-2);
  }
  .seg button.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .list {
    flex: 1;
    min-height: 0;
    overflow-y: auto;
    padding: var(--space-1) var(--space-2) var(--space-2);
  }
  .heading {
    padding: var(--space-3) var(--space-2) var(--space-1);
    font-size: var(--fs-xs);
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-2);
  }
  .row {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 32px;
    padding: 0 var(--space-2);
    border-radius: var(--radius-s);
    color: var(--text-1);
    cursor: pointer;
  }
  .row.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .title {
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }
  mark {
    background: none;
    color: var(--text-0);
    font-weight: 600;
  }
  .group {
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .keys {
    display: flex;
    align-items: center;
    gap: var(--space-1);
    flex: none;
  }
  .combo {
    display: flex;
    gap: 2px;
  }
  .or {
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  kbd {
    min-width: 20px;
    height: 20px;
    padding: 0 5px;
    display: inline-grid;
    place-items: center;
    border-radius: 4px;
    border: 1px solid var(--line-strong);
    background: var(--bg-2);
    color: var(--text-1);
    font: 500 var(--fs-xs) / 1 var(--font-ui);
  }
  .sheet {
    columns: 2 300px;
    column-gap: var(--space-5);
    padding: var(--space-2) var(--space-4) var(--space-4);
  }
  section {
    break-inside: avoid;
    padding-bottom: var(--space-3);
  }
  h3 {
    margin: var(--space-2) 0 var(--space-1);
    font-size: var(--fs-xs);
    font-weight: 500;
    text-transform: uppercase;
    letter-spacing: 0.08em;
    color: var(--text-2);
  }
  .shortcut {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    min-height: 28px;
    color: var(--text-1);
  }
  .none {
    margin: 0;
    padding: var(--space-5) var(--space-2);
    text-align: center;
    color: var(--text-2);
  }
  .foot {
    display: flex;
    align-items: center;
    gap: var(--space-4);
    padding: var(--space-2) var(--space-4);
    border-top: 1px solid var(--line);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .foot kbd {
    min-width: 16px;
    height: 16px;
    margin-right: 2px;
    font-size: 10px;
  }
  .foot .end {
    margin-left: auto;
  }
</style>
