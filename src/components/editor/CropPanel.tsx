import { FlipHorizontal2, FlipVertical2, RotateCcw, RotateCw } from "lucide-react";
import { DEFAULTS, GEOMETRY, same } from "../../adjustments";
import { flipped, heldAspect, quarterTurn, Rect, shrinkToFit, straightened, toCrop, toRect, turnedSize, withAspect } from "../../crop";
import { useStore } from "../../store";
import { Slider } from "./Slider";

/** Shapes to crop to, as width over height the long way. `null` leaves the shape free. */
const SHAPES: { label: string; ratio: number | null | "original" }[] = [
  { label: "Original", ratio: "original" },
  { label: "Free", ratio: null },
  { label: "1:1", ratio: 1 },
  { label: "4:3", ratio: 4 / 3 },
  { label: "3:2", ratio: 3 / 2 },
  { label: "5:4", ratio: 5 / 4 },
  { label: "16:9", ratio: 16 / 9 },
];

/** A small picture of a shape, drawn the way up the crop is. */
function ShapeIcon({ ratio, tall }: { ratio: number | null; tall: boolean }) {
  const box = 18;
  // Free has no fixed shape: a dashed square says so.
  const r = ratio === null ? 1 : tall ? Math.min(ratio, 1 / ratio) : Math.max(ratio, 1 / ratio);
  const [w, h] = r >= 1 ? [box, box / r] : [box * r, box];
  return (
    <svg className="shape-icon" width={box + 2} height={box + 2} aria-hidden>
      <rect x={1 + (box - w) / 2} y={1 + (box - h) / 2} width={w} height={h} rx={1.5} className={ratio === null ? "free" : ""} />
    </svg>
  );
}

export function CropPanel() {
  const adjustments = useStore((s) => s.editor.adjustments);
  const photo = useStore((s) => s.editor.size);
  const ready = useStore((s) => s.editor.ready);
  const chosenShape = useStore((s) => s.cropShape);
  const chosenAspect = useStore((s) => s.cropAspect);
  const s = useStore.getState();
  if (!photo || !ready) return <div className="crop-panel waiting" />;

  const turned = turnedSize(photo, adjustments.rotation);
  const rect = toRect(adjustments.crop, turned);
  const changed = GEOMETRY.some((key) => !same(adjustments[key], DEFAULTS[key]));
  // The chosen shape only holds while the crop still has it: undo can put back a crop of another shape.
  const aspect = heldAspect(rect, chosenAspect);
  const shape = aspect === null ? "Free" : chosenShape;

  const setRect = (next: Rect) => {
    s.adjust({ crop: toCrop(next, turned) });
    s.commitAdjust();
  };

  const chooseShape = (label: string, ratio: number | null | "original") => {
    if (ratio === null) {
      s.setCropShape(label, null);
      return;
    }
    // Keep the crop the way up it already is: a tall crop stays tall.
    const tall = rect.height > rect.width;
    const next = ratio === "original" ? turned.width / turned.height : tall ? 1 / ratio : ratio;
    s.setCropShape(label, next);
    setRect(withAspect(rect, next, adjustments.straighten, turned));
  };

  const swapSides = () => {
    if (aspect !== null) {
      s.setCropShape(shape, 1 / aspect);
      setRect(withAspect(rect, 1 / aspect, adjustments.straighten, turned));
    } else {
      // Let go of a shape that no longer held, so the swapped crop can't take it up again.
      if (chosenAspect !== null) s.setCropShape("Free", null);
      setRect(shrinkToFit({ ...rect, width: rect.height, height: rect.width }, adjustments.straighten, turned));
    }
  };

  const turn = (clockwise: boolean) => {
    s.adjust(quarterTurn(adjustments, clockwise));
    s.commitAdjust();
    s.setCropShape(shape, aspect === null ? null : 1 / aspect);
  };

  const flip = (horizontal: boolean) => {
    s.adjust(flipped(adjustments, horizontal));
    s.commitAdjust();
  };

  const reset = () => {
    s.adjust(Object.fromEntries(GEOMETRY.map((key) => [key, DEFAULTS[key]])));
    s.commitAdjust();
    s.setCropShape("Free", null);
  };

  const tall = rect.height > rect.width;
  const square = aspect !== null && Math.abs(aspect - 1) < 1e-6;
  const ratioOf = (option: (typeof SHAPES)[number]) =>
    option.ratio === "original" ? turned.width / turned.height : option.ratio;
  const angle = adjustments.straighten;

  return (
    <div className="crop-panel">
      <section className="crop-group">
        <h3 className="crop-heading">Aspect ratio</h3>
        <div className="shape-grid" role="radiogroup" aria-label="Aspect ratio">
          {SHAPES.map((option) => (
            <button
              key={option.label}
              role="radio"
              aria-checked={shape === option.label}
              className={`shape ${shape === option.label ? "active" : ""}`}
              onClick={() => chooseShape(option.label, option.ratio)}
            >
              <ShapeIcon ratio={ratioOf(option)} tall={tall} />
              {option.label}
            </button>
          ))}
        </div>
        <div className="segmented" role="radiogroup" aria-label="Orientation">
          {[false, true].map((portrait) => (
            <button
              key={String(portrait)}
              role="radio"
              aria-checked={tall === portrait}
              className={tall === portrait ? "active" : ""}
              // A square crop is the same either way up.
              disabled={square}
              onClick={() => tall !== portrait && swapSides()}
            >
              {portrait ? "Portrait" : "Landscape"}
            </button>
          ))}
        </div>
      </section>

      <section className="crop-group">
        <Slider
          label="Straighten"
          value={angle}
          min={-45}
          max={45}
          step={0.1}
          format={(value) => `${value.toFixed(1)}°`}
          onChange={(next) => s.adjust({ straighten: next, crop: toCrop(straightened(rect, angle, next, turned), turned) })}
        />
      </section>

      <section className="crop-group">
        <h3 className="crop-heading">Rotate and flip</h3>
        <div className="crop-buttons">
          <button className="button" onClick={() => turn(false)}>
            <RotateCcw size={15} /> Rotate left
          </button>
          <button className="button" onClick={() => turn(true)}>
            <RotateCw size={15} /> Rotate right
          </button>
          <button className="button" onClick={() => flip(true)}>
            {/* Lucide names these after the mirror line, not the direction of the flip. */}
            <FlipVertical2 size={15} /> Flip horizontal
          </button>
          <button className="button" onClick={() => flip(false)}>
            <FlipHorizontal2 size={15} /> Flip vertical
          </button>
        </div>
      </section>

      <p className="crop-size">
        {Math.round(rect.width)} × {Math.round(rect.height)} pixels
      </p>

      <div className="crop-actions">
        <button className="button quiet" disabled={!changed} onClick={reset}>
          Reset
        </button>
        <button className="button primary" title="Done (Enter)" onClick={() => s.setSidePanel("adjust")}>
          Done
        </button>
      </div>
    </div>
  );
}
