import { ArrowLeftRight, FlipHorizontal2, FlipVertical2, RotateCcw, RotateCw } from "lucide-react";
import { DEFAULTS, GEOMETRY } from "../../adjustments";
import { flipped, quarterTurn, Rect, shrinkToFit, straightened, toCrop, toRect, turnedSize, withAspect } from "../../crop";
import { useStore } from "../../store";
import { Slider } from "./Slider";

/** Shapes to crop to, as width over height for a landscape picture. `null` leaves the shape free. */
const SHAPES: { label: string; ratio: number | null | "original" }[] = [
  { label: "Free", ratio: null },
  { label: "Original", ratio: "original" },
  { label: "1:1", ratio: 1 },
  { label: "5:4", ratio: 5 / 4 },
  { label: "4:3", ratio: 4 / 3 },
  { label: "3:2", ratio: 3 / 2 },
  { label: "16:9", ratio: 16 / 9 },
];

/**
 * The crop tool's controls. While this panel is open the photo is shown
 * whole, with the crop frame over it.
 */
export function CropPanel() {
  const adjustments = useStore((s) => s.editor.adjustments);
  const photo = useStore((s) => s.editor.size);
  const ready = useStore((s) => s.editor.ready);
  const shape = useStore((s) => s.cropShape);
  const aspect = useStore((s) => s.cropAspect);
  const s = useStore.getState();
  if (!photo || !ready) return <div className="crop-panel waiting" />;

  const turned = turnedSize(photo, adjustments.rotation);
  const rect = toRect(adjustments.crop, turned);
  const changed = GEOMETRY.some((key) => JSON.stringify(adjustments[key]) !== JSON.stringify(DEFAULTS[key]));

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
      setRect(shrinkToFit({ ...rect, width: rect.height, height: rect.width }, adjustments.straighten, turned));
    }
  };

  const turn = (clockwise: boolean) => {
    s.adjust(quarterTurn(adjustments, clockwise));
    s.commitAdjust();
    if (aspect !== null) s.setCropShape(shape, 1 / aspect);
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

  return (
    <div className="crop-panel">
      <p className="panel-hint">Drag the corners or edges of the frame, or drag inside it to move.</p>

      <div className="tool-row">
        <span className="tool-row-label">Shape</span>
        <button className="icon-button small" title="Swap width and height" aria-label="Swap width and height" onClick={swapSides}>
          <ArrowLeftRight size={14} />
        </button>
      </div>
      <div className="chips" role="radiogroup" aria-label="Crop shape">
        {SHAPES.map((option) => (
          <button
            key={option.label}
            role="radio"
            aria-checked={shape === option.label}
            className={`chip ${shape === option.label ? "active" : ""}`}
            onClick={() => chooseShape(option.label, option.ratio)}
          >
            {option.label}
          </button>
        ))}
      </div>

      <Slider
        label="Straighten"
        value={adjustments.straighten}
        min={-45}
        max={45}
        step={0.1}
        onChange={(angle) =>
          s.adjust({ straighten: angle, crop: toCrop(straightened(rect, adjustments.straighten, angle, turned), turned) })
        }
      />

      <div className="tool-row">
        <span className="tool-row-label">Turn and flip</span>
      </div>
      <div className="button-row">
        <button className="button" title="Turn left" aria-label="Turn left" onClick={() => turn(false)}>
          <RotateCcw size={15} />
        </button>
        <button className="button" title="Turn right" aria-label="Turn right" onClick={() => turn(true)}>
          <RotateCw size={15} />
        </button>
        <button className="button" title="Flip left to right" aria-label="Flip left to right" onClick={() => flip(true)}>
          {/* Lucide names these after the mirror line, not the direction of the flip. */}
          <FlipVertical2 size={15} />
        </button>
        <button className="button" title="Flip top to bottom" aria-label="Flip top to bottom" onClick={() => flip(false)}>
          <FlipHorizontal2 size={15} />
        </button>
      </div>

      <div className="crop-actions">
        <button className="button quiet" disabled={!changed} onClick={reset}>
          Reset crop
        </button>
        <button className="button primary" onClick={() => s.setSidePanel("adjust")}>
          Done
        </button>
      </div>
    </div>
  );
}
