import { ReactNode, useState } from "react";
import { ChevronDown, Mountain } from "lucide-react";
import { LABELS, LocalKey, Mask, MaskPart, noLocalAdjustments, rangeOf } from "../../adjustments";
import { canAdd, describeLocal, MaskStart, MAX_MASKS, SHAPE_NAMES, startKind, tidy } from "../../masks";
import { useStore } from "../../store";
import { MaskList, SHAPE_ICONS } from "./MaskList";
import { Slider } from "./Slider";

/** The ways to start a mask: a short name for its tile, and a line saying what it picks out. */
const STARTS: { start: MaskStart; name: string; about: string }[] = [
  { start: "subject", name: "Subject", about: "The person or thing the photo is of, found for you" },
  { start: "background", name: "Background", about: "Everything but the subject" },
  { start: "sky", name: "Sky", about: "The sky, found for you" },
  { start: "object", name: "Object", about: "Draw a loop around something, and its outline is found for you" },
  { start: "brush", name: "Brush", about: "Paint over the part you want to change" },
  { start: "linear", name: "Linear", about: "Fade a change across the photo, like darkening a sky" },
  { start: "radial", name: "Radial", about: "A soft oval, to lift a face or draw the eye" },
  { start: "luminance", name: "Range", about: "Only the darkest or brightest parts of the photo" },
];

/** Starting a mask: a tile for each way of picking out part of the photo. */
function NewMask({ first }: { first: boolean }) {
  const masks = useStore((s) => s.editor.adjustments.masks);
  const finding = useStore((s) => s.editor.finding);
  const circling = useStore((s) => s.editor.circling);
  const [chosen, setChosen] = useState<MaskStart | null>(null);
  const s = useStore.getState();
  const full = masks.length >= MAX_MASKS;
  const drawing = circling !== null && circling.mode === null && !finding;
  const hint = full
    ? `A photo can have ${MAX_MASKS} masks. Delete one to start another.`
    : drawing
      ? "Draw a loop around the thing you want on the photo. It needn’t be neat."
      : finding && chosen
        ? `Finding ${finding}…`
        : first
          ? "Change one part of the photo on its own. Choose how to pick it out:"
          : null;
  return (
    <section className="new-mask" aria-label="New mask">
      {!first && <h3 className="panel-heading">New mask</h3>}
      {hint && <p className="panel-hint">{hint}</p>}
      <div className="new-mask-grid">
        {STARTS.map(({ start, name, about }) => {
          const kind = startKind(start);
          const Icon = start === "background" ? Mountain : SHAPE_ICONS[kind];
          const waiting = (finding !== null || drawing) && chosen === start;
          return (
            <button
              key={start}
              className={`new-mask-tile ${waiting ? "engaged" : ""}`}
              title={about}
              disabled={full || finding !== null || (drawing && start !== "object") || !canAdd(masks, kind)}
              aria-busy={waiting && finding !== null}
              aria-pressed={drawing && start === "object"}
              onClick={() => {
                if (drawing) return s.cancelCircle();
                setChosen(start);
                s.addMask(start);
              }}
            >
              {waiting && finding !== null ? <span className="spinner" /> : <Icon size={18} strokeWidth={1.6} />}
              <span>{name}</span>
            </button>
          );
        })}
      </div>
      {drawing && (
        <div className="part-footer">
          <span />
          <button className="button quiet" onClick={() => s.cancelCircle()}>
            Cancel
          </button>
        </div>
      )}
    </section>
  );
}

/** A mask's sliders, grouped as in the adjust panel. */
const GROUPS: { title: string; keys: LocalKey[] }[] = [
  { title: "Light", keys: ["exposure", "contrast", "highlights", "shadows", "whites", "blacks"] },
  { title: "Color", keys: ["temperature", "tint", "vibrance", "saturation"] },
  { title: "Detail", keys: ["clarity", "dehaze", "sharpening", "noiseReduction"] },
];

/** The mask as of the last step in the history: what a change is measured against. */
const committedMask = (id: number) => useStore.getState().editor.committed.masks.find((mask) => mask.id === id);

function Group({ title, action, children }: { title: string; action?: ReactNode; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  return (
    <section className={`tool-section ${open ? "open" : ""}`}>
      <header>
        <button className="section-title" aria-expanded={open} onClick={() => setOpen(!open)}>
          <ChevronDown size={14} />
          {title}
        </button>
        {action}
      </header>
      {open && <div className="section-body">{children}</div>}
    </section>
  );
}

/** Settings of the part chosen in the layers: the brush, a radial's softness, or a brightness range. */
function PartSettings({ mask, part, index }: { mask: Mask; part: MaskPart; index: number }) {
  const brush = useStore((s) => s.brush);
  const s = useStore.getState();
  const { shape } = part;
  const label = (what: string) => () => `${mask.name}: ${what}`;
  const percent = (value: number) => `${Math.round(value)}%`;
  const title = SHAPE_NAMES[shape.kind];

  if (shape.kind === "brush") {
    return (
      <Group title={title}>
        <div className="segmented" role="radiogroup" aria-label="Brush">
          {[false, true].map((erase) => (
            <button
              key={String(erase)}
              role="radio"
              aria-checked={brush.erase === erase}
              className={brush.erase === erase ? "active" : ""}
              onClick={() => s.setBrush({ erase })}
            >
              {erase ? "Erase" : "Paint"}
            </button>
          ))}
        </div>
        <Slider label="Size" value={brush.size} min={1} max={100} origin={30} format={String} onChange={(size) => s.setBrush({ size })} />
        <Slider label="Feather" value={brush.feather} min={0} max={100} origin={50} format={percent} onChange={(feather) => s.setBrush({ feather })} />
        <Slider label="Strength" value={brush.strength} min={1} max={100} origin={100} format={percent} onChange={(strength) => s.setBrush({ strength })} />
        <div className="part-footer">
          <p className="panel-hint">Hold Alt to erase. [ and ] change the size.</p>
          <button
            className="button quiet"
            disabled={shape.strokes.length === 0}
            onClick={() => s.updateMaskPart(index, { ...part, shape: { kind: "brush", strokes: [] } }, `${mask.name}: clear brush`)}
          >
            Clear
          </button>
        </div>
      </Group>
    );
  }
  // Found parts and linear gradients have nothing to set here: a line on how to work with them.
  if (shape.kind === "subject" || shape.kind === "sky") {
    return <p className="panel-hint part-hint">Found again on each photo these edits are pasted onto. Subtract a brush to tidy it.</p>;
  }
  if (shape.kind === "object") {
    return (
      <div className="part-footer part-hint">
        <p className="panel-hint">Found inside the loop you drew.</p>
        <button className="button quiet" onClick={() => s.startCircle({ mode: part.mode, replace: index })}>
          Draw again
        </button>
      </div>
    );
  }
  if (shape.kind === "linear") {
    return <p className="panel-hint part-hint">Drag the ends on the photo. The change is full at the solid line and fades out by the faint one.</p>;
  }
  if (shape.kind === "radial") {
    return (
      <Group title={title}>
        <Slider
          label="Feather"
          value={Math.round(shape.feather * 100)}
          min={0}
          max={100}
          origin={50}
          format={percent}
          onChange={(value) => s.updateMaskPart(index, { ...part, shape: { ...shape, feather: value / 100 } })}
          commitLabel={label("feather")}
        />
        <p className="panel-hint">Drag the edge to resize it and the outer knob to turn it.</p>
      </Group>
    );
  }
  const range = (key: "low" | "high" | "smoothness", value: number) => {
    const next = { ...shape, [key]: tidy(value / 100) };
    // The two ends never cross.
    if (key === "low") next.high = Math.max(next.high, next.low);
    if (key === "high") next.low = Math.min(next.low, next.high);
    s.updateMaskPart(index, { ...part, shape: next });
  };
  return (
    <Group title={title}>
      <Slider
        label="Darkest"
        value={Math.round(shape.low * 100)}
        min={0}
        max={100}
        origin={0}
        format={percent}
        onChange={(v) => range("low", v)}
        commitLabel={label("brightness range")}
      />
      <Slider
        label="Brightest"
        value={Math.round(shape.high * 100)}
        min={0}
        max={100}
        origin={100}
        format={percent}
        onChange={(v) => range("high", v)}
        commitLabel={label("brightness range")}
      />
      <Slider
        label="Smoothness"
        value={Math.round(shape.smoothness * 100)}
        min={0}
        max={100}
        origin={15}
        format={percent}
        onChange={(v) => range("smoothness", v)}
        commitLabel={label("brightness range")}
      />
    </Group>
  );
}

/** The chosen mask: its chosen part's settings, then the sliders that apply inside it. */
function MaskSettings({ mask, partIndex }: { mask: Mask; partIndex: number | null }) {
  const s = useStore.getState();
  const part = partIndex !== null ? mask.parts[partIndex] : undefined;
  const changed = GROUPS.some((group) => group.keys.some((key) => mask.adjustments[key] !== 0));
  const setLocal = (key: LocalKey, value: number) => s.updateMask(mask.id, { adjustments: { ...mask.adjustments, [key]: value } });

  return (
    <div className="mask-settings">
      {part && partIndex !== null && <PartSettings mask={mask} part={part} index={partIndex} />}
      {GROUPS.map((group, i) => (
        <Group
          key={group.title}
          title={group.title}
          action={
            i === 0 &&
            changed && (
              <button className="section-reset" onClick={() => s.updateMask(mask.id, { adjustments: noLocalAdjustments() }, `${mask.name}: reset`)}>
                Reset
              </button>
            )
          }
        >
          {group.keys.map((key) => {
            const { min, max, step } = rangeOf(key);
            return (
              <Slider
                key={key}
                label={LABELS[key]}
                value={mask.adjustments[key]}
                // Less sharpening than the rest of the photo is a fair thing to ask for in one place.
                min={key === "sharpening" ? -100 : min}
                max={max}
                step={step}
                onChange={(value) => setLocal(key, value)}
                commitLabel={() => {
                  const now = useStore.getState().editor.adjustments.masks.find((m) => m.id === mask.id);
                  return now ? describeLocal(now, committedMask(mask.id)?.adjustments ?? noLocalAdjustments()) : undefined;
                }}
              />
            );
          })}
        </Group>
      ))}
    </div>
  );
}

/**
 * Masks: local adjustments. One column, top to bottom: the photo's masks
 * (the chosen one open to show its parts), then either the tiles that start
 * a new mask, or the chosen part's settings and the sliders that apply
 * inside the mask.
 */
export function MasksPanel() {
  const masks = useStore((s) => s.editor.adjustments.masks);
  const ready = useStore((s) => s.editor.ready && s.editor.photoId === s.openId);
  const maskId = useStore((s) => s.editor.maskId);
  const partIndex = useStore((s) => s.editor.partIndex);
  const mask = masks.find((m) => m.id === maskId);

  return (
    <div className={`adjust masks ${ready ? "" : "waiting"}`} inert={!ready}>
      {masks.length > 0 && <MaskList />}
      {mask ? <MaskSettings mask={mask} partIndex={partIndex} /> : <NewMask first={masks.length === 0} />}
    </div>
  );
}
