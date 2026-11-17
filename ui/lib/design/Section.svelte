<!--
  Collapsible panel section. Open/closed is remembered per title.
  - `edited` shows a dot next to the title; with `onreset`, a reset button appears on hover.
  - `storeKey` remembers open/closed under its own name (two panels with a "Light" section).
-->
<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    open?: boolean;
    edited?: boolean;
    onreset?: () => void;
    storeKey?: string;
    children: Snippet;
  }
  let { title, open = $bindable(true), edited = false, onreset, storeKey, children }: Props = $props();

  const key = () => `tonality.section.${storeKey ?? title}`;
  try {
    const v = localStorage.getItem(key());
    if (v !== null) open = v === "1";
  } catch {
    /* per-viewer convenience only */
  }

  function toggle() {
    open = !open;
    try {
      localStorage.setItem(key(), open ? "1" : "0");
    } catch {
      /* ignore */
    }
  }
</script>

<section class:open>
  <div class="head">
    <button class="toggle" onclick={toggle} aria-expanded={open}>
      <svg class="chev" viewBox="0 0 16 16" width="12" height="12" aria-hidden="true"><path d="m6 4 4 4-4 4" /></svg>
      <span class="title">{title}</span>
      {#if edited}<span class="dot" title="Edited"></span>{/if}
    </button>
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
    visibility: hidden;
    margin-right: var(--space-1);
    padding: 2px 8px;
    border-radius: var(--radius-s);
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
  .head:hover .reset {
    visibility: visible;
  }
  .reset:hover {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .body {
    display: grid;
    gap: var(--space-2);
    padding: var(--space-1) var(--space-2) var(--space-4);
  }
</style>
