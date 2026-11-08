<script lang="ts">
  import type { EditState } from "$lib/bindings/EditState";
  import type { ImageInfo } from "$lib/bindings/ImageInfo";
  import Section from "$lib/design/Section.svelte";
  import Slider from "$lib/design/Slider.svelte";

  interface Props {
    edit: EditState;
    image: ImageInfo;
    /**
     * Called once per completed gesture. `key` names the control so the history can merge
     * rapid repeats (arrow-key nudges); `null` for one-off actions.
     */
    oncommit?: (key: string | null) => void;
  }
  let { edit = $bindable(), image, oncommit }: Props = $props();

  type LightKey = keyof EditState["light"];
  const lightControls: { key: LightKey; label: string; min: number; max: number; step: number; decimals: number }[] = [
    { key: "exposure", label: "Exposure", min: -5, max: 5, step: 0.01, decimals: 2 },
    { key: "contrast", label: "Contrast", min: -100, max: 100, step: 1, decimals: 0 },
    { key: "highlights", label: "Highlights", min: -100, max: 100, step: 1, decimals: 0 },
    { key: "shadows", label: "Shadows", min: -100, max: 100, step: 1, decimals: 0 },
    { key: "whites", label: "Whites", min: -100, max: 100, step: 1, decimals: 0 },
    { key: "blacks", label: "Blacks", min: -100, max: 100, step: 1, decimals: 0 },
  ];

  const wb = $derived(edit.whiteBalance);
  const isAsShot = $derived(wb.temperature === null && wb.tint === null);

  function resetWb() {
    edit.whiteBalance = { temperature: null, tint: null };
    oncommit?.(null);
  }
</script>

<div class="panel">
  <Section title="White Balance">
    <div class="wb-mode">
      <button class:active={isAsShot} onclick={resetWb} disabled={isAsShot}>As Shot</button>
      <span class="hint">{isAsShot ? "" : "Custom"}</span>
    </div>
    <Slider
      label="Temperature"
      value={wb.temperature ?? image.asShotTemperature}
      min={2000}
      max={12000}
      step={50}
      defaultValue={image.asShotTemperature}
      unit="K"
      bipolar={false}
      oninput={(v) => (edit.whiteBalance.temperature = v)}
      oncommit={() => oncommit?.("temperature")}
      onreset={() => {
        edit.whiteBalance.temperature = null;
        oncommit?.(null);
      }}
    />
    <Slider
      label="Tint"
      value={wb.tint ?? image.asShotTint}
      min={-150}
      max={150}
      defaultValue={image.asShotTint}
      oninput={(v) => (edit.whiteBalance.tint = v)}
      oncommit={() => oncommit?.("tint")}
      onreset={() => {
        edit.whiteBalance.tint = null;
        oncommit?.(null);
      }}
    />
  </Section>

  <Section title="Light">
    {#each lightControls as c (c.key)}
      <Slider
        label={c.label}
        value={edit.light[c.key]}
        min={c.min}
        max={c.max}
        step={c.step}
        decimals={c.decimals}
        oninput={(v) => (edit.light[c.key] = v)}
        oncommit={() => oncommit?.(c.key)}
      />
    {/each}
  </Section>
</div>

<style>
  .panel {
    height: 100%;
    overflow-y: auto;
  }
  .wb-mode {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    margin-bottom: var(--space-2);
  }
  .wb-mode button {
    font-size: var(--fs-xs);
    padding: 2px 8px;
    border-radius: var(--radius-s);
    background: var(--bg-2);
    color: var(--text-1);
  }
  .wb-mode button.active {
    background: var(--bg-3);
    color: var(--text-0);
  }
  .wb-mode button:disabled {
    cursor: default;
  }
  .hint {
    font-size: var(--fs-xs);
    color: var(--text-2);
  }
</style>
