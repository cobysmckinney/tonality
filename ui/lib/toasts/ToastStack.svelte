<script lang="ts">
  import { flip } from "svelte/animate";
  import { fly } from "svelte/transition";
  import { revealItemInDir } from "@tauri-apps/plugin-opener";
  import { api } from "$lib/api";
  import { toasts, type Toast } from "./toasts.svelte";

  let stack = $state<HTMLElement>();
  let hovered = $state(false);
  let focused = $state(false);
  /** The toast whose text was just copied. */
  let copied = $state<number | null>(null);
  let copiedTimer: ReturnType<typeof setTimeout> | undefined;

  $effect(() => {
    if (hovered || focused) toasts.hold();
    else toasts.release();
  });

  async function copy(t: Toast) {
    try {
      await navigator.clipboard.writeText(t.text);
      copied = t.id;
      clearTimeout(copiedTimer);
      copiedTimer = setTimeout(() => (copied = null), 1500);
    } catch (e) {
      console.warn("[toasts] copy:", e);
    }
  }

  async function openLog() {
    try {
      const path = await api.logPath();
      if (path) await revealItemInDir(path);
      else toasts.notice("There's no log file: it couldn't be opened at launch");
    } catch (e) {
      console.warn("[toasts] open log:", e);
    }
  }

  // Capture, and mounted before the dialogs: Esc on a toast dismisses it rather than closing
  // the dialog behind it.
  function onKey(e: KeyboardEvent) {
    if (e.key !== "Escape" || !stack?.contains(e.target as Node)) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    const last = toasts.list.at(-1);
    if (last) close(last.id);
  }

  /** Dismiss from inside the stack. The focused toast leaves the page without a focusout, so
   *  focus moves to the newest one left (Esc again dismisses that), or off the stack. */
  function close(id: number) {
    const inside = stack?.contains(document.activeElement);
    const rest = stack?.querySelectorAll<HTMLElement>(`.toast:not([data-id="${id}"]) .close`);
    toasts.dismiss(id);
    if (!inside) return;
    const next = [...(rest ?? [])].at(-1);
    if (next) next.focus();
    else {
      (document.activeElement as HTMLElement | null)?.blur();
      focused = false;
    }
  }

  // What the live region says: each notice once, and again, in full, when a repeat bumps its
  // count (the "×2" alone would be read as "2 times" with no message).
  let live = $state("");
  const said = new Map<number, number>();
  let saying: ReturnType<typeof setTimeout> | undefined;
  $effect(() => {
    for (const t of toasts.list) {
      if (t.kind !== "notice" || said.get(t.id) === t.count) continue;
      said.set(t.id, t.count);
      const text = t.count > 1 ? `${t.text} (${t.count} times)` : t.text;
      // Emptied first: the same words twice running would otherwise be no change at all.
      live = "";
      clearTimeout(saying);
      saying = setTimeout(() => (live = text), 50);
    }
    for (const id of said.keys()) if (!toasts.list.some((t) => t.id === id)) said.delete(id);
  });
</script>

<svelte:window onkeydowncapture={onKey} />

<!-- Notices are read out from here. A polite live region is only reliably announced when it
     is already on the page and its text then changes, which a toast that arrives complete with
     its own role="status" is not. (Errors are role="alert", which is announced on arrival.) -->
<div class="sr-only" aria-live="polite" aria-atomic="true">{live}</div>

<section
  class="toasts"
  aria-label="Notifications"
  bind:this={stack}
  onpointerenter={() => (hovered = true)}
  onpointerleave={() => (hovered = false)}
  onfocusin={() => (focused = true)}
  onfocusout={(e) => (focused = stack?.contains(e.relatedTarget as Node) ?? false)}
>
  {#each toasts.list as t (t.id)}
    <div
      class="toast {t.kind}"
      data-id={t.id}
      role={t.kind === "error" ? "alert" : undefined}
      animate:flip={{ duration: 160 }}
      transition:fly={{ x: 24, duration: 160 }}
    >
      {#if t.kind === "error"}
        <svg class="mark" viewBox="0 0 16 16" width="14" height="14" aria-hidden="true">
          <circle cx="8" cy="8" r="6.5" />
          <path d="M8 4.75v4M8 11.25v.01" />
        </svg>
      {/if}
      <div class="body">
        <p class="text">
          {t.text}
          {#if t.count > 1}<span class="count" aria-label="{t.count} times">×{t.count}</span>{/if}
        </p>
        {#if t.kind === "error"}
          <div class="actions">
            <button class="link" onclick={() => copy(t)}>
              {copied === t.id ? "Copied" : "Copy"}
            </button>
            <button
              class="link"
              onclick={openLog}
              title="Errors are recorded in this file. Attach it when you report a problem."
            >
              Open Log
            </button>
          </div>
        {/if}
      </div>
      <button class="icon-btn close" onclick={() => close(t.id)} aria-label="Dismiss">
        <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
          <path d="M4 4l8 8M12 4l-8 8" />
        </svg>
      </button>
    </div>
  {/each}
</section>

<style>
  .toasts {
    position: fixed;
    right: calc(var(--gutter) + var(--space-3));
    bottom: calc(var(--gutter) + var(--space-3));
    z-index: 200; /* above dialogs and drag ghosts, below the splash */
    display: flex;
    flex-direction: column;
    align-items: flex-end;
    gap: var(--space-2);
    width: min(380px, calc(100vw - 2 * var(--space-4)));
    pointer-events: none;
  }
  .toast {
    pointer-events: auto;
    display: flex;
    align-items: flex-start;
    gap: var(--space-2);
    width: 100%;
    padding: var(--space-2) var(--space-2) var(--space-2) var(--space-3);
    border-radius: var(--radius-m);
    background: var(--bg-2);
    box-shadow: var(--shadow-pop);
    font-size: var(--fs-s);
    color: var(--text-0);
  }
  .toast.notice {
    width: auto;
    max-width: 100%;
  }
  .mark {
    flex: none;
    margin-top: 5px;
    fill: none;
    stroke: var(--danger);
    stroke-width: 1.5;
    stroke-linecap: round;
  }
  .body {
    flex: 1;
    min-width: 0;
    padding: 4px 0;
  }
  .text {
    margin: 0;
    line-height: 1.45;
    overflow-wrap: anywhere;
    white-space: pre-wrap;
    user-select: text;
    /* A long backtrace mustn't push the stack off the screen. */
    max-height: 8lh;
    overflow-y: auto;
  }
  .count {
    margin-left: var(--space-1);
    padding: 0 6px;
    border-radius: var(--radius-pill);
    background: var(--bg-4);
    color: var(--text-1);
    font-family: var(--font-num);
    font-size: var(--fs-xs);
  }
  .actions {
    display: flex;
    gap: var(--space-3);
    margin-top: var(--space-1);
  }
  .link {
    color: var(--text-1);
    font-size: var(--fs-xs);
  }
  .link:hover {
    color: var(--text-0);
    text-decoration: underline;
  }
  .close {
    flex: none;
  }
</style>
