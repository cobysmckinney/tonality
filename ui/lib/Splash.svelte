<!--
  Startup animation: the wordmark is traced stroke by stroke on a blank screen, fills in, then
  flies to the top bar's wordmark while the screen fades away to show the app.
  Skipped for reduced motion; a click or key press skips it too.
-->
<script lang="ts">
  import { onMount } from "svelte";
  import { app } from "$lib/app.svelte";
  import { WORDMARK } from "$lib/design/brand";

  // Each contour (a letter, a counter, the i's dot) traces on its own, left to right.
  const contours = WORDMARK.d.split(/(?=M)/).filter((s) => s.trim());

  const TRACE = 1100; // ms for one contour
  const STAGGER = 1100; // ms from the first contour starting to the last
  const FILL = 350;
  const HOLD = 150;
  const MOVE = 700;
  const EASE = "cubic-bezier(0.65, 0, 0.25, 1)";

  let overlay: HTMLDivElement;
  let backdrop: HTMLDivElement;
  let word: SVGSVGElement;
  let filled = $state(false);
  let skipped = false;
  const running: Animation[] = [];

  const wait = (ms: number) => new Promise((r) => setTimeout(r, ms));

  function finish() {
    for (const a of running) a.cancel();
    app.splash = false;
  }

  function skip() {
    if (skipped) return;
    skipped = true;
    // A short fade rather than a cut, so the app doesn't flash in.
    overlay
      .animate([{ opacity: 1 }, { opacity: 0 }], { duration: 160, fill: "forwards" })
      .finished.then(finish);
  }

  async function play() {
    const paths = Array.from(word.querySelectorAll("path.trace"));
    const step = paths.length > 1 ? STAGGER / (paths.length - 1) : 0;
    paths.forEach((p, i) =>
      running.push(
        p.animate([{ strokeDashoffset: 1 }, { strokeDashoffset: 0 }], {
          duration: TRACE,
          delay: i * step,
          easing: "cubic-bezier(0.45, 0, 0.3, 1)",
          fill: "both",
        }),
      ),
    );
    await wait(STAGGER + TRACE * 0.8);
    if (skipped) return;
    filled = true;
    await wait(FILL + HOLD);
    if (skipped) return;

    // Fly to the top bar's wordmark (which stays hidden until we land on it).
    const target = document.querySelector<SVGSVGElement>("[data-brand-wordmark] svg");
    if (target) {
      const from = word.getBoundingClientRect();
      const to = target.getBoundingClientRect();
      const s = to.width / from.width;
      const dx = to.left - from.left;
      const dy = to.top - from.top;
      running.push(
        word.animate(
          [{ transform: "none" }, { transform: `translate(${dx}px, ${dy}px) scale(${s})` }],
          {
            duration: MOVE,
            easing: EASE,
            fill: "forwards",
          },
        ),
      );
    }
    const fade = backdrop.animate([{ opacity: 1 }, { opacity: 0 }], {
      duration: MOVE * 0.75,
      delay: MOVE * 0.25,
      easing: "ease-out",
      fill: "forwards",
    });
    running.push(fade);
    await fade.finished.catch(() => {});
    if (!skipped) finish();
  }

  onMount(() => {
    if (matchMedia("(prefers-reduced-motion: reduce)").matches) {
      finish();
      return;
    }
    play();
  });
</script>

<svelte:window onkeydowncapture={skip} />

<!-- svelte-ignore a11y_click_events_have_key_events, a11y_no_static_element_interactions -->
<div class="splash" bind:this={overlay} onpointerdown={skip} aria-hidden="true">
  <div class="backdrop" bind:this={backdrop}></div>
  <svg
    bind:this={word}
    class:filled
    viewBox="0 0 {WORDMARK.width} {WORDMARK.height}"
    width="440"
    height={(440 * WORDMARK.height) / WORDMARK.width}
  >
    <!-- Filled as one path so the counters (o, a) stay open; traced contour by contour. -->
    <path class="fill" d={WORDMARK.d} />
    {#each contours as d, i (i)}<path class="trace" {d} pathLength="1" />{/each}
  </svg>
</div>

<style>
  .splash {
    position: fixed;
    inset: 0;
    z-index: 1000;
    display: grid;
    place-items: center;
    color: var(--text-0);
  }
  .backdrop {
    position: absolute;
    inset: 0;
    background: var(--bg-0);
  }
  svg {
    position: relative;
    max-width: 60vw;
    height: auto;
    overflow: visible;
    transform-origin: 0 0;
  }
  .fill {
    fill: currentColor;
    opacity: 0;
    transition: opacity 350ms ease-out;
  }
  .filled .fill {
    opacity: 1;
  }
  .trace {
    fill: none;
    stroke: currentColor;
    /* In wordmark units: about 1.2 px at the splash's size. */
    stroke-width: 9;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-dasharray: 1 1;
    stroke-dashoffset: 1;
    transition: stroke-opacity 350ms ease-out;
  }
  .filled .trace {
    stroke-opacity: 0;
  }
</style>
