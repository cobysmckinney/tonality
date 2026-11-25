<!--
  Collapsible panel section. Open/closed is remembered per title (across photos and restarts);
  with "Open one section at a time" (Settings), opening one closes the others.
  - `edited` shows a dot next to the title; with `onreset`, a reset button appears on hover.
  - `storeKey` remembers open/closed under its own name (two panels with a "Light" section).
  - `actions`: small buttons in the header (Light's Auto).
-->
<script lang="ts">
  import { untrack, type Snippet } from "svelte";
  import { rememberOpen, sections, wasOpen } from "./sections.svelte";

  interface Props {
    title: string;
    open?: boolean;
    edited?: boolean;
    onreset?: () => void;
    storeKey?: string;
    actions?: Snippet;
    children: Snippet;
  }
  let {
    title,
    open = $bindable(true),
    edited = false,
    onreset,
    storeKey,
    actions,
    children,
  }: Props = $props();

  const key = () => storeKey ?? title;
  open = wasOpen(key(), open);

  function toggle() {
    open = !open;
    rememberOpen(key(), open);
    if (open) sections.open(key());
  }

  // One at a time: another section opening (while this one is shown) closes this one. Openings
  // from before it was shown (in another panel) don't count.
  const shownAt = untrack(() => sections.opened?.n ?? 0);
  $effect(() => {
    const o = sections.opened;
    if (!o || o.n <= shownAt || !sections.oneAtATime || o.key === key()) return;
    untrack(() => {
      if (!open) return;
      open = false;
      rememberOpen(key(), false);
    });
  });
</script>

<section class:open>
  <div class="head">
    <button class="toggle" onclick={toggle} aria-expanded={open}>
      <svg class="chev" viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
        <path d="m6 4 4 4-4 4" />
      </svg>
      <span class="title">{title}</span>
      {#if edited}<span class="dot" title="Edited"></span>{/if}
    </button>
    {#if actions}<div class="actions">{@render actions()}</div>{/if}
    {#if edited && onreset}
      <button class="reset" onclick={onreset} title="Reset {title}">Reset</button>
    {/if}
  </div>
  {#if open}
    <div class="body">{@render children()}</div>
  {/if}
</section>

<style>
  section {
    margin: 0 var(--space-2);
    border-radius: var(--radius-m);
  }
  section + :global(section) {
    margin-top: 2px;
  }
  .head {
    display: flex;
    align-items: center;
    border-radius: var(--radius-m);
  }
  .head:hover {
    background: var(--bg-2);
  }
  .toggle {
    flex: 1;
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 38px;
    padding: 0 var(--space-2);
    font-weight: 500;
    font-size: var(--fs-m);
    text-align: left;
  }
  .chev {
    flex: none;
    fill: none;
    stroke: var(--text-2);
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
    transition: transform 150ms var(--ease);
  }
  .open .chev {
    transform: rotate(90deg);
  }
  .title {
    color: var(--text-1);
  }
  .open .title,
  .head:hover .title {
    color: var(--text-0);
  }
  .dot {
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--text-1);
  }
  .reset {
    /* Hidden but still focusable, so Tab reaches it. */
    opacity: 0;
    margin-right: var(--space-1);
    padding: 2px 8px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .head:hover .reset,
  .head:focus-within .reset {
    opacity: 1;
  }
  .reset:hover {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .actions {
    display: flex;
    gap: 2px;
    margin-right: var(--space-1);
  }
  /* Small buttons, like `.btn` at header size: they must look clickable at a glance. */
  .actions :global(button) {
    min-width: 40px;
    height: 22px;
    padding: 0 8px;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    border: 1px solid var(--line-strong);
    font-size: var(--fs-xs);
    font-weight: 500;
    color: var(--text-1);
    transition:
      background 120ms var(--ease),
      color 120ms var(--ease);
  }
  .actions :global(button:hover:not(:disabled)) {
    background: var(--bg-4);
    color: var(--text-0);
  }
  .actions :global(button:disabled) {
    opacity: 0.45;
  }
  .body {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2) var(--space-4);
  }
</style>
