<!--
  tonality's logo: the italic `t` mark or the "tonality" wordmark, in the current text color.
  Artwork comes from scripts/brand.py.
-->
<script lang="ts">
  import { MARK, WORDMARK } from "./brand";

  interface Props {
    kind?: "mark" | "wordmark";
    /** Height of the ink in px; width follows. */
    height: number;
    /** Accessible name; leave empty when the logo sits next to the same words. */
    label?: string;
  }
  let { kind = "wordmark", height, label = "" }: Props = $props();

  const art = $derived(kind === "mark" ? MARK : WORDMARK);
</script>

<svg
  viewBox="0 0 {art.width} {art.height}"
  {height}
  width={(height * art.width) / art.height}
  role={label ? "img" : undefined}
  aria-label={label || undefined}
  aria-hidden={label ? undefined : "true"}
>
  <path d={art.d} />
</svg>

<style>
  svg {
    display: block;
    flex: none;
    fill: currentColor;
  }
</style>
