<!-- Collapsible panel section. All sections are visible at once; collapsing is only for focus. -->
<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    title: string;
    open?: boolean;
    children: Snippet;
  }
  let { title, open = $bindable(true), children }: Props = $props();
</script>

<section>
  <button class="head" onclick={() => (open = !open)} aria-expanded={open}>
    <span>{title}</span>
    <span class="chev" class:open>›</span>
  </button>
  {#if open}
    <div class="body">{@render children()}</div>
  {/if}
</section>

<style>
  section {
    border-bottom: 1px solid var(--line);
  }
  .head {
    width: 100%;
    display: flex;
    justify-content: space-between;
    align-items: center;
    padding: var(--space-3) var(--space-4);
    font-weight: 500;
    font-size: var(--fs-m);
  }
  .head:hover {
    background: var(--bg-2);
  }
  .chev {
    color: var(--text-2);
    transition: transform 150ms var(--ease);
  }
  .chev.open {
    transform: rotate(90deg);
  }
  .body {
    padding: 0 var(--space-4) var(--space-3);
  }
</style>
