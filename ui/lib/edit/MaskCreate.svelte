<!--
  The ways to make a new mask: shapes drawn by hand, and areas found automatically.
  Used as the side panel's empty state and as the mask list's "+" menu.
-->
<script lang="ts">
  import { AI_ICON, AI_LABEL, MAX_MASKS, SHAPE_ICON, type AiPreset, type MaskKind } from "./masks";
  import { session } from "./session.svelte";

  interface Props {
    /** Called after an option is chosen (closes a menu). */
    onpicked?: () => void;
  }
  let { onpicked }: Props = $props();

  const shapes: { kind: MaskKind; label: string; title: string }[] = [
    { kind: "linear", label: "Linear Gradient", title: "A gradual fade across the photo, like a sky" },
    { kind: "radial", label: "Radial Gradient", title: "An ellipse, soft at its edge" },
    { kind: "brush", label: "Brush", title: "Paint where it applies" },
  ];
  const detects: { id: AiPreset; title: string }[] = [
    { id: "subject", title: "The photo's main subject" },
    { id: "background", title: "Everything but the main subject" },
    { id: "sky", title: "The sky" },
    { id: "people", title: "Every person in the photo" },
    { id: "object", title: "Circle something to select it" },
  ];

  const full = $derived(session.masks.length >= MAX_MASKS);

  function shape(k: MaskKind) {
    session.addMask(k);
    onpicked?.();
  }
  function detect(p: AiPreset) {
    if (p === "object") session.startLasso();
    else session.addAiMask(p);
    onpicked?.();
  }
</script>

<div class="create">
  <span class="eyebrow">Draw</span>
  {#each shapes as s (s.kind)}
    <button onclick={() => shape(s.kind)} disabled={full} title={s.title}>
      <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"><path d={SHAPE_ICON[s.kind]} /></svg>
      {s.label}
    </button>
  {/each}
  <span class="eyebrow">Find automatically</span>
  {#each detects as d (d.id)}
    <button
      class:on={d.id === "object" && session.lassoing !== null}
      onclick={() => detect(d.id)}
      disabled={full}
      title={d.title}
    >
      <svg viewBox="0 0 16 16" width="15" height="15" aria-hidden="true"><path d={AI_ICON[d.id]} /></svg>
      {d.id === "object" ? "Select Object" : AI_LABEL[d.id]}
    </button>
  {/each}
  {#if full}<p class="note">Up to {MAX_MASKS} masks per photo.</p>{/if}
</div>

<style>
  .create {
    display: grid;
    gap: 2px;
  }
  .eyebrow {
    padding: var(--space-2) var(--space-2) var(--space-1);
  }
  .eyebrow:first-child {
    padding-top: 0;
  }
  button {
    display: flex;
    align-items: center;
    gap: var(--space-3);
    height: 32px;
    padding: 0 var(--space-2);
    border-radius: var(--radius-s);
    color: var(--text-1);
    font-size: var(--fs-s);
    text-align: left;
  }
  button:hover:not(:disabled),
  button.on {
    background: var(--bg-3);
    color: var(--text-0);
  }
  button:disabled {
    opacity: 0.4;
  }
  svg {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.4;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .note {
    margin: var(--space-2) var(--space-2) 0;
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
</style>
