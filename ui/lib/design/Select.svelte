<!--
  Compact dropdown (single choice): a button showing the chosen option, opening a listbox.
  - Keyboard: Enter, Space or ↑/↓ open it; ↑/↓, Home/End move; Enter or Space chooses; Esc or Tab
    closes without choosing; typing letters jumps to the option starting with them.
  - `onpreview(id)` fires as the pointer or the keyboard lands on an option while open, so the
    caller can show it before it's chosen; `onpreview(null)` when it closes without a choice.
  - `onchoose(id)` fires once, when an option is chosen (also when it's the current one).
-->
<script lang="ts" module>
  export interface SelectOption {
    id: string;
    label: string;
    /** Dim text at the right of the option (e.g. "5500 K"). */
    detail?: string;
    title?: string;
  }
  let nextId = 0;
</script>

<script lang="ts">
  import { tick } from "svelte";
  import { moveTo, typeahead, TYPEAHEAD_MS } from "./listbox";

  interface Props {
    options: SelectOption[];
    value: string;
    /** Accessible name of the control. */
    label: string;
    disabled?: boolean;
    title?: string;
    onpreview?: (id: string | null) => void;
    onchoose: (id: string) => void;
  }
  let { options, value, label, disabled = false, title, onpreview, onchoose }: Props = $props();

  const uid = `select-${nextId++}`;
  let open = $state(false);
  let active = $state(0);
  let trigger: HTMLButtonElement;
  let list = $state<HTMLDivElement | null>(null);
  let typed = "";
  let typedAt = 0;

  const current = $derived(options.find((o) => o.id === value));
  const selectedIndex = $derived(
    Math.max(
      0,
      options.findIndex((o) => o.id === value),
    ),
  );

  function show() {
    if (disabled || open) return;
    open = true;
    active = selectedIndex;
    typed = "";
    // Focus the list once it's in the DOM, so the keys below reach it.
    tick().then(() => list?.focus());
  }

  function close(chosen: string | null) {
    if (!open) return;
    open = false;
    if (chosen === null) onpreview?.(null);
    else onchoose(chosen);
    trigger?.focus();
  }

  function setActive(i: number) {
    if (i === active || !options[i]) return;
    active = i;
    onpreview?.(options[i].id);
    list?.querySelector(`#${uid}-${i}`)?.scrollIntoView({ block: "nearest" });
  }

  function onTriggerKey(e: KeyboardEvent) {
    if (["ArrowDown", "ArrowUp", "Enter", " "].includes(e.key)) {
      e.preventDefault();
      show();
    }
  }

  function onListKey(e: KeyboardEvent) {
    // Keys stay here: the editor's shortcuts must not see them while the list is open.
    e.stopPropagation();
    const to = moveTo(e.key, active, options.length);
    if (to !== null) {
      e.preventDefault();
      return setActive(to);
    }
    if (e.key === "Enter" || e.key === " ") {
      e.preventDefault();
      return close(options[active].id);
    }
    if (e.key === "Escape") {
      e.preventDefault();
      return close(null);
    }
    if (e.key === "Tab") return close(null);
    if (e.key.length === 1 && !e.ctrlKey && !e.metaKey && !e.altKey) {
      e.preventDefault();
      const now = performance.now();
      typed = now - typedAt > TYPEAHEAD_MS ? e.key : typed + e.key;
      typedAt = now;
      const i = typeahead(
        options.map((o) => o.label),
        typed,
        active,
      );
      if (i !== null) setActive(i);
    }
  }

  // A press anywhere else closes it without choosing, and goes no further: dismissing the list
  // over the photo must not also zoom it, nor start dragging a slider.
  function onWindowDown(e: PointerEvent) {
    if (!open) return;
    const t = e.target as Node;
    if (list?.contains(t) || trigger.contains(t)) return;
    e.stopPropagation();
    close(null);
  }
</script>

<svelte:window onpointerdowncapture={onWindowDown} />

<div class="select">
  <button
    bind:this={trigger}
    class="trigger"
    class:open
    {disabled}
    {title}
    aria-label={label}
    aria-haspopup="listbox"
    aria-expanded={open}
    aria-controls={open ? `${uid}-list` : undefined}
    onclick={() => (open ? close(null) : show())}
    onkeydown={onTriggerKey}
  >
    <span class="value">{current?.label ?? ""}</span>
    <svg viewBox="0 0 16 16" width="12" height="12" aria-hidden="true">
      <path d="m4 6 4 4 4-4" />
    </svg>
  </button>
  {#if open}
    <!-- Focus lives on the list; the active option is named by aria-activedescendant. -->
    <div
      bind:this={list}
      id="{uid}-list"
      class="list"
      role="listbox"
      tabindex="-1"
      aria-label={label}
      aria-activedescendant="{uid}-{active}"
      onkeydown={onListKey}
      onfocusout={(e) => {
        const to = e.relatedTarget as Node | null;
        if (to && !list?.contains(to) && to !== trigger) close(null);
      }}
    >
      {#each options as o, i (o.id)}
        <!-- svelte-ignore a11y_click_events_have_key_events (the listbox handles keys) -->
        <div
          id="{uid}-{i}"
          class="option"
          class:active={i === active}
          class:selected={o.id === value}
          role="option"
          aria-selected={o.id === value}
          tabindex="-1"
          title={o.title}
          onpointerenter={() => setActive(i)}
          onclick={() => close(o.id)}
        >
          <span class="check" aria-hidden="true">
            {#if o.id === value}
              <svg viewBox="0 0 16 16" width="11" height="11"><path d="m3.5 8.5 3 3 6-7" /></svg>
            {/if}
          </span>
          <span class="label">{o.label}</span>
          {#if o.detail}<span class="detail">{o.detail}</span>{/if}
        </div>
      {/each}
    </div>
  {/if}
</div>

<style>
  .select {
    position: relative;
    min-width: 0;
  }
  .trigger {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    width: 100%;
    height: 28px;
    padding: 0 var(--space-2) 0 var(--space-3);
    border-radius: var(--radius-s);
    background: var(--bg-2);
    color: var(--text-0);
    font-size: var(--fs-s);
    transition: background 120ms var(--ease);
  }
  .trigger:hover:not(:disabled),
  .trigger.open {
    background: var(--bg-3);
  }
  .trigger:disabled {
    opacity: 0.45;
    cursor: default;
  }
  .value {
    flex: 1;
    min-width: 0;
    text-align: left;
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  svg {
    flex: none;
    fill: none;
    stroke: currentColor;
    stroke-width: 1.6;
    stroke-linecap: round;
    stroke-linejoin: round;
  }
  .trigger svg {
    color: var(--text-2);
  }
  .list {
    position: absolute;
    z-index: 20;
    top: calc(100% + 4px);
    left: 0;
    right: 0;
    max-height: 320px;
    overflow-y: auto;
    padding: 4px;
    border-radius: var(--radius-m);
    background: var(--bg-2);
    box-shadow: var(--shadow-pop);
    outline: none;
  }
  .option {
    display: flex;
    align-items: center;
    gap: var(--space-2);
    height: 26px;
    padding: 0 var(--space-2);
    border-radius: var(--radius-s);
    font-size: var(--fs-s);
    color: var(--text-1);
    cursor: pointer;
  }
  .option.active {
    background: var(--bg-4);
    color: var(--text-0);
  }
  .option.selected {
    color: var(--text-0);
  }
  .check {
    display: grid;
    place-items: center;
    width: 12px;
    flex: none;
  }
  .label {
    flex: 1;
    white-space: nowrap;
  }
  .detail {
    font-family: var(--font-num);
    font-size: 10px;
    color: var(--text-2);
  }
</style>
