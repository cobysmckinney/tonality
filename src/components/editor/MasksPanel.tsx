import { useState } from "react";
import { CircleDashed, Ellipsis, Eye, EyeOff, Paintbrush, Plus, Rows3, SunMedium, X } from "lucide-react";
import { LABELS, LocalKey, Mask, MaskMode, MaskPart, noLocalAdjustments, rangeOf, Shape } from "../../adjustments";
import { canAdd, describeLocal, MAX_MASKS, MODE_NAMES, SHAPE_NAMES, tidy } from "../../masks";
import { MenuEntry, useStore } from "../../store";
import { menuBelow } from "../Toolbar";
import { NameInput } from "./NameInput";
import { Slider } from "./Slider";

const ICONS = { brush: Paintbrush, linear: Rows3, radial: CircleDashed, luminance: SunMedium };
const KINDS: Shape["kind"][] = ["brush", "linear", "radial", "luminance"];

/** A mask's sliders, grouped as in the adjust panel. */
const GROUPS: { title: string; keys: LocalKey[] }[] = [
  { title: "Light", keys: ["exposure", "contrast", "highlights", "shadows", "whites", "blacks"] },
  { title: "Color", keys: ["temperature", "tint", "vibrance", "saturation"] },
  { title: "Detail", keys: ["clarity", "dehaze", "sharpening", "noiseReduction"] },
];

/** The mask as of the last step in the history: what a change is measured against. */
const committedMask = (id: number) => useStore.getState().editor.committed.masks.find((mask) => mask.id === id);

function MaskRow(props: { mask: Mask; selected: boolean; renaming: boolean; onRename: (renaming: boolean) => void }) {
  const { mask, selected } = props;
  const s = useStore.getState();
  if (props.renaming) {
    return (
      <li className="mask-row">
        <NameInput
          name={mask.name}
          label="Mask name"
          onDone={(name) => {
            props.onRename(false);
            const tidied = name.trim();
            if (tidied && tidied !== mask.name) s.updateMask(mask.id, { name: tidied }, `Rename ${mask.name} to ${tidied}`);
          }}
        />
      </li>
    );
  }
  const entries: MenuEntry[] = [
    { label: "Rename", run: () => props.onRename(true) },
    { label: mask.invert ? "Don’t invert" : "Invert", run: () => s.updateMask(mask.id, { invert: !mask.invert }, `${mask.name}: invert`) },
    "separator",
    { label: "Delete", danger: true, run: () => s.removeMask(mask.id) },
  ];
  const Icon = ICONS[mask.parts[0]?.shape.kind ?? "brush"];
  return (
    <li className={`mask-row ${selected ? "selected" : ""} ${mask.visible ? "" : "hidden"}`}>
      <button
        className="mask-main"
        aria-pressed={selected}
        onClick={() => s.selectMask(selected ? null : mask.id)}
        onContextMenu={(event) => {
          event.preventDefault();
          s.openMenu(event.clientX, event.clientY, entries);
        }}
      >
        <Icon size={14} />
        <span className="mask-name">{mask.name}</span>
      </button>
      <button
        className="icon-button small"
        title={mask.visible ? "Hide this mask’s effect" : "Show this mask’s effect"}
        aria-label={mask.visible ? `Hide ${mask.name}` : `Show ${mask.name}`}
        aria-pressed={!mask.visible}
        onClick={() => s.updateMask(mask.id, { visible: !mask.visible }, `${mask.visible ? "Hide" : "Show"} ${mask.name}`)}
      >
        {mask.visible ? <Eye size={14} /> : <EyeOff size={14} />}
      </button>
      <button className="icon-button small" title="Rename, invert or delete" aria-label={`More for ${mask.name}`} onClick={(event) => menuBelow(event, entries)}>
        <Ellipsis size={14} />
      </button>
    </li>
  );
}

function PartRow(props: { mask: Mask; part: MaskPart; index: number; selected: boolean }) {
  const { mask, part, index, selected } = props;
  const s = useStore.getState();
  const Icon = ICONS[part.shape.kind];
  const modes: MenuEntry[] = (["add", "subtract", "intersect"] as MaskMode[]).map((mode) => ({
    label: MODE_NAMES[mode],
    checked: part.mode === mode,
    run: () => s.updateMaskPart(index, { ...part, mode }, `${mask.name}: ${MODE_NAMES[mode].toLowerCase()} ${SHAPE_NAMES[part.shape.kind].toLowerCase()}`),
  }));
  return (
    <li className={`mask-part-row ${selected ? "selected" : ""}`}>
      <button className="mask-main" aria-pressed={selected} onClick={() => s.selectMask(mask.id, index)}>
        <Icon size={14} />
        <span className="mask-name">{SHAPE_NAMES[part.shape.kind]}</span>
      </button>
      {/* The first part starts the mask, so how it combines with what came before means nothing. */}
      {index > 0 && (
        <button className="mode-button" title="How this part combines with the ones above it" onClick={(event) => menuBelow(event, modes)}>
          {MODE_NAMES[part.mode]}
        </button>
      )}
      <button className="icon-button small" title="Remove this part" aria-label={`Remove ${SHAPE_NAMES[part.shape.kind]}`} onClick={() => s.removeMaskPart(index)}>
        <X size={14} />
      </button>
    </li>
  );
}

/** Settings of the part chosen: the brush, or the shape's softness, or a brightness range. */
function PartSettings({ mask, part, index }: { mask: Mask; part: MaskPart; index: number }) {
  const brush = useStore((s) => s.brush);
  const s = useStore.getState();
  const { shape } = part;
  const label = (what: string) => () => `${mask.name}: ${what}`;
  const percent = (value: number) => `${Math.round(value)}%`;

  if (shape.kind === "brush") {
    return (
      <div className="mask-settings">
        <p className="panel-hint">Paint on the photo. Hold Alt to erase; [ and ] change the size.</p>
        <Slider label="Size" value={brush.size} min={1} max={100} origin={30} format={String} onChange={(size) => s.setBrush({ size })} />
        <Slider label="Feather" value={brush.feather} min={0} max={100} origin={50} format={percent} onChange={(feather) => s.setBrush({ feather })} />
        <Slider label="Strength" value={brush.strength} min={1} max={100} origin={100} format={percent} onChange={(strength) => s.setBrush({ strength })} />
        <div className="chips">
          <button className={`chip ${brush.erase ? "" : "active"}`} aria-pressed={!brush.erase} onClick={() => s.setBrush({ erase: false })}>
            Paint
          </button>
          <button className={`chip ${brush.erase ? "active" : ""}`} aria-pressed={brush.erase} onClick={() => s.setBrush({ erase: true })}>
            Erase
          </button>
          <button
            className="chip"
            disabled={shape.strokes.length === 0}
            onClick={() => s.updateMaskPart(index, { ...part, shape: { kind: "brush", strokes: [] } }, `${mask.name}: clear brush`)}
          >
            Clear
          </button>
        </div>
      </div>
    );
  }
  if (shape.kind === "linear") {
    return <p className="panel-hint mask-settings">Drag the ends on the photo: full effect at the solid line, none at the faint one.</p>;
  }
  if (shape.kind === "radial") {
    return (
      <div className="mask-settings">
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
        <p className="panel-hint">Drag the edge to resize, the outer knob to turn it.</p>
      </div>
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
    <div className="mask-settings">
      <Slider label="Darkest" value={Math.round(shape.low * 100)} min={0} max={100} origin={0} format={percent} onChange={(v) => range("low", v)} commitLabel={label("brightness range")} />
      <Slider label="Brightest" value={Math.round(shape.high * 100)} min={0} max={100} origin={100} format={percent} onChange={(v) => range("high", v)} commitLabel={label("brightness range")} />
      <Slider label="Smoothness" value={Math.round(shape.smoothness * 100)} min={0} max={100} origin={15} format={percent} onChange={(v) => range("smoothness", v)} commitLabel={label("brightness range")} />
    </div>
  );
}

function MaskDetail({ mask, partIndex }: { mask: Mask; partIndex: number | null }) {
  const masks = useStore((s) => s.editor.adjustments.masks);
  const showMask = useStore((s) => s.editor.showMask);
  const s = useStore.getState();
  const part = partIndex !== null ? mask.parts[partIndex] : undefined;

  const addMenu = (event: React.MouseEvent) => {
    const entry = (kind: Shape["kind"], mode: MaskMode): MenuEntry => ({
      label: `${MODE_NAMES[mode]} ${SHAPE_NAMES[kind].toLowerCase()}`,
      disabled: !canAdd(masks, kind),
      run: () => s.addMaskPart(kind, mode),
    });
    menuBelow(event, [
      ...KINDS.map((kind) => entry(kind, "add")),
      "separator",
      ...KINDS.map((kind) => entry(kind, "subtract")),
      "separator",
      entry("luminance", "intersect"),
    ]);
  };

  const setLocal = (key: LocalKey, value: number) =>
    s.updateMask(mask.id, { adjustments: { ...mask.adjustments, [key]: value } });
  const changed = GROUPS.some((group) => group.keys.some((key) => mask.adjustments[key] !== 0));

  return (
    <div className="mask-detail">
      <h3 className="preset-heading">
        Parts
        <button className="icon-button small" title="Add to or take away from this mask" aria-label="Add a part" onClick={addMenu}>
          <Plus size={15} />
        </button>
      </h3>
      <ul className="mask-parts">
        {mask.parts.map((p, index) => (
          <PartRow key={index} mask={mask} part={p} index={index} selected={index === partIndex} />
        ))}
      </ul>
      {part && partIndex !== null && <PartSettings mask={mask} part={part} index={partIndex} />}

      <div className="chips mask-options">
        <button
          className={`chip ${mask.invert ? "active" : ""}`}
          aria-pressed={mask.invert}
          onClick={() => s.updateMask(mask.id, { invert: !mask.invert }, `${mask.name}: invert`)}
        >
          Invert
        </button>
        <button className={`chip ${showMask ? "active" : ""}`} aria-pressed={showMask} title="Tint the mask on the photo (O)" onClick={s.toggleMaskOverlay}>
          Show overlay
        </button>
      </div>

      {GROUPS.map((group, i) => (
        <section key={group.title} className="mask-group">
          <h3 className="preset-heading">
            {group.title}
            {i === 0 && changed && (
              <button
                className="section-reset"
                onClick={() => s.updateMask(mask.id, { adjustments: noLocalAdjustments() }, `${mask.name}: reset`)}
              >
                Reset
              </button>
            )}
          </h3>
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
        </section>
      ))}
    </div>
  );
}

/**
 * Masks: local adjustments. Each mask is an area of the photo, built from
 * parts (brush strokes, gradients, a brightness range), with its own sliders
 * that add to the photo's own wherever it covers.
 */
export function MasksPanel() {
  const masks = useStore((s) => s.editor.adjustments.masks);
  const ready = useStore((s) => s.editor.ready && s.editor.photoId === s.openId);
  const maskId = useStore((s) => s.editor.maskId);
  const partIndex = useStore((s) => s.editor.partIndex);
  const [renaming, setRenaming] = useState<number | null>(null);
  const s = useStore.getState();
  const mask = masks.find((m) => m.id === maskId);

  const newMenu = (event: React.MouseEvent) =>
    menuBelow(
      event,
      KINDS.map((kind) => ({
        label: SHAPE_NAMES[kind],
        disabled: masks.length >= MAX_MASKS || !canAdd(masks, kind),
        run: () => s.addMask(kind),
      })),
    );

  return (
    <div className={`masks ${ready ? "" : "waiting"}`} inert={!ready}>
      <div className="mask-scroll">
        <h3 className="preset-heading">
          Masks
          <button className="icon-button small" title="New mask" aria-label="New mask" onClick={newMenu}>
            <Plus size={15} />
          </button>
        </h3>
        <ul className="mask-list">
          {masks.map((m) => (
            <MaskRow
              key={m.id}
              mask={m}
              selected={m.id === maskId}
              renaming={renaming === m.id}
              onRename={(yes) => setRenaming(yes ? m.id : null)}
            />
          ))}
        </ul>
        {masks.length === 0 && (
          <p className="preset-empty">Change one part of the photo: paint it, or lay a gradient or a circle over it. Start with +.</p>
        )}
        {mask && <MaskDetail mask={mask} partIndex={partIndex} />}
      </div>
    </div>
  );
}
