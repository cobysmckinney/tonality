import { useMemo } from "react";
import { useStore } from "../../store";

const WIDTH = 256;
const HEIGHT = 100;

/** An area path for one 256-bin channel, scaled so `ceiling` reaches the top. */
function area(bins: Uint32Array, ceiling: number): string {
  let path = `M0 ${HEIGHT}`;
  for (let i = 0; i < 256; i++) {
    // A square root keeps small counts visible next to a tall peak.
    const height = Math.min(1, Math.sqrt(bins[i] / ceiling)) * HEIGHT;
    path += `L${i} ${(HEIGHT - height).toFixed(1)}L${i + 1} ${(HEIGHT - height).toFixed(1)}`;
  }
  return `${path}L${WIDTH} ${HEIGHT}Z`;
}

/**
 * Brightness distribution of the edited photo, shadows on the left. The
 * corner markers light up when shadows or highlights are clipping; clicking
 * one shows where on the photo.
 */
export function Histogram() {
  const histogram = useStore((s) => s.editor.histogram);
  const highlights = useStore((s) => s.editor.highlightsClipped);
  const shadows = useStore((s) => s.editor.shadowsClipped);
  const showClipping = useStore((s) => s.editor.showClipping);
  const toggle = useStore((s) => s.toggleClipping);

  const paths = useMemo(() => {
    if (!histogram) return null;
    const channels = [0, 1, 2].map((c) => histogram.subarray(c * 256, c * 256 + 256));
    // Scale to the tallest bin away from the very ends, so a clipped spike doesn't flatten the rest.
    let ceiling = 1;
    for (const bins of channels) for (let i = 2; i < 254; i++) ceiling = Math.max(ceiling, bins[i]);
    return channels.map((bins) => area(bins, ceiling));
  }, [histogram]);

  return (
    <section className="panel histogram" aria-label="Histogram">
      <svg viewBox={`0 0 ${WIDTH} ${HEIGHT}`} preserveAspectRatio="none" aria-hidden>
        {paths && (
          <>
            <path d={paths[0]} className="red" />
            <path d={paths[1]} className="green" />
            <path d={paths[2]} className="blue" />
          </>
        )}
      </svg>
      <button
        className={`clip-marker shadows ${shadows ? "clipping" : ""} ${showClipping ? "showing" : ""}`}
        title={shadows ? "Shadows are clipping. Show where (J)" : "Show clipping (J)"}
        aria-label="Show clipped shadows and highlights"
        aria-pressed={showClipping}
        onClick={toggle}
      />
      <button
        className={`clip-marker highlights ${highlights ? "clipping" : ""} ${showClipping ? "showing" : ""}`}
        title={highlights ? "Highlights are clipping. Show where (J)" : "Show clipping (J)"}
        aria-label="Show clipped shadows and highlights"
        aria-pressed={showClipping}
        onClick={toggle}
      />
    </section>
  );
}
