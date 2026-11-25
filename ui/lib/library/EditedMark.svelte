<!-- The "edited" mark on a thumbnail: two sliders, for a photo whose shown version isn't as-shot. -->
<script lang="ts">
  let { size = 12 }: { size?: number } = $props();
</script>

<span class="edited" title="Edited" aria-label="Edited">
  <svg viewBox="0 0 12 12" width={size} height={size} aria-hidden="true">
    <path d="M1.5 4h9M1.5 8h9" />
    <circle cx="4" cy="4" r="1.6" />
    <circle cx="8" cy="8" r="1.6" />
  </svg>
</span>

<style>
  .edited {
    display: grid;
    place-items: center;
    padding: 2px;
    border-radius: 9px;
    background: rgb(0 0 0 / 0.55);
    color: #fff;
  }
  svg {
    display: block;
  }
  path {
    stroke: currentColor;
    stroke-width: 1.2;
    stroke-linecap: round;
    fill: none;
    opacity: 0.8;
  }
  circle {
    fill: currentColor;
  }
</style>
