import { useRef, useState } from "react";
import { FlipHorizontal2, FlipVertical2, RotateCcw, RotateCw } from "lucide-react";
import { DEFAULTS, GEOMETRY, same } from "../../adjustments";
import {
  flipped,
  heldAspect,
  isTall,
  parseRatio,
  quarterTurn,
  Rect,
  shapeAspect,
  shrinkToFit,
  straightened,
  toCrop,
  toRect,
  turnedSize,
  withAspect,
} from "../../crop";
import { useStore } from "../../store";
import { Slider } from "./Slider";

/** Shapes to crop to, as width over height the long way. `null` leaves the shape free; Custom is whatever is typed. */
const SHAPES: { label: string; ratio: number | null | "original" | "custom" }[] = [
  { label: "Original", ratio: "original" },
  { label: "Free", ratio: null },
  { label: "1:1", ratio: 1 },
  { label: "4:3", ratio: 4 / 3 },
  { label: "3:2", ratio: 3 / 2 },
  { label: "5:4", ratio: 5 / 4 },
  { label: "16:9", ratio: 16 / 9 },
  { label: "Custom", ratio: "custom" },
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
  const custom = useStore((s) => s.cropCustom);
  // The field for a shape of your own shows once Custom is clicked, and while it is the shape.
  const [typing, setTyping] = useState(false);
  const [draft, setDraft] = useState(custom);
  const field = useRef<HTMLInputElement>(null);
  // Escape in the field leaves what was typed unused.
  const reverting = useRef(false);
  const s = useStore.getState();
  if (!photo || !ready) return <div className="crop-panel waiting" />;

  const turned = turnedSize(photo, adjustments.rotation);
  const rect = toRect(adjustments.crop, turned);
  const changed = GEOMETRY.some((key) => !same(adjustments[key], DEFAULTS[key]));
  // The chosen shape only holds while the crop still has it: undo can put back a crop of another shape.
  const aspect = heldAspect(rect, chosenAspect);
  const shape = aspect === null ? "Free" : chosenShape;

  // Each change is a step for undo in the crop tool; the history gets one step for the whole visit, on Done.
  const setRect = (next: Rect) => {
    s.adjust({ crop: toCrop(next, turned) });
    s.markCrop();
  };

  const chooseShape = (label: string, ratio: number | null | "original") => {
    if (label !== "Custom") setTyping(false);
    if (ratio === null) {
      s.setCropShape(label, null);
      return;
    }
    // Keep the crop the way up it already is: a tall crop stays tall, even for "Original".
    const next = shapeAspect(ratio === "original" ? turned.width / turned.height : ratio, rect, turned);
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

  /** Crops to the shape typed for Custom, if it is one. */
  const applyCustom = (text: string) => {
    const ratio = parseRatio(text);
    if (ratio === null) return;
    s.setCropCustom(text.trim());
    chooseShape("Custom", ratio);
  };

  // Leaving the field as it was doesn't reshape a crop already that shape.
  const keepDraft = () => {
    if (reverting.current) reverting.current = false;
    else if (draft.trim() !== custom || shape !== "Custom") applyCustom(draft);
  };

  const chooseCustom = () => {
    setTyping(true);
    setDraft(custom);
    applyCustom(custom);
    // The field appears with this click; focus it once it has.
    requestAnimationFrame(() => field.current?.select());
  };

  const turn = (clockwise: boolean) => {
    s.adjust(quarterTurn(adjustments, clockwise));
    s.markCrop();
    s.setCropShape(shape, aspect === null ? null : 1 / aspect);
  };

  const flip = (horizontal: boolean) => {
    s.adjust(flipped(adjustments, horizontal));
    s.markCrop();
  };

  const reset = () => {
    s.adjust(Object.fromEntries(GEOMETRY.map((key) => [key, DEFAULTS[key]])));
    s.markCrop();
    s.setCropShape("Free", null);
  };

  const tall = isTall(rect, turned);
  const square = aspect !== null && Math.abs(aspect - 1) < 1e-6;
  const customRatio = parseRatio(custom);
  const ratioOf = (option: (typeof SHAPES)[number]) =>
    option.ratio === "original" ? turned.width / turned.height : option.ratio === "custom" ? customRatio : option.ratio;
  const showField = typing || shape === "Custom";
  const draftWrong = draft.trim() !== "" && parseRatio(draft) === null;
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
              onClick={() =>
                option.ratio === "custom" ? chooseCustom() : chooseShape(option.label, option.ratio)
              }
            >
              <ShapeIcon ratio={ratioOf(option)} tall={tall} />
              {option.label}
            </button>
          ))}
        </div>
        {showField && (
          <form
            className="crop-custom"
            onSubmit={(event) => {
              event.preventDefault();
              keepDraft();
            }}
          >
            <label htmlFor="crop-custom">Width : height</label>
            <input
              ref={field}
              id="crop-custom"
              className={`film-input ${draftWrong ? "wrong" : ""}`}
              value={draft}
              placeholder="6:7"
              spellCheck={false}
              aria-invalid={draftWrong}
              title="A shape of your own, such as 6:7 or 65:24"
              onChange={(event) => setDraft(event.currentTarget.value)}
              onBlur={keepDraft}
              onKeyDown={(event) => {
                // Escape leaves the field as it was, without leaving the crop tool.
                if (event.key === "Escape") {
                  reverting.current = true;
                  setDraft(custom);
                  event.currentTarget.blur();
                }
              }}
            />
          </form>
        )}
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
          onCommit={s.markCrop}
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
        <div className="crop-finish">
          <button className="button" title="Put the crop back as it was (Esc)" onClick={s.cancelCrop}>
            Cancel
          </button>
          <button className="button primary" title="Keep the crop (Enter)" onClick={() => s.setSidePanel("adjust")}>
            Done
          </button>
        </div>
      </div>
    </div>
  );
}
