import { useEffect, useLayoutEffect, useRef, useState } from "react";
import { ChevronDown, Ellipsis, Eye, EyeOff, Plus, X } from "lucide-react";
import { Mask, MaskMode, MaskPart, Shape } from "../../adjustments";
import { api } from "../../api";
import { canAdd, MODE_NAMES, SHAPE_NAMES } from "../../masks";
import { MenuEntry, useStore } from "../../store";
import { menuBelow } from "../Toolbar";
import { SHAPE_ICONS } from "./MasksPanel";
import { NameInput } from "./NameInput";

const KINDS: Shape["kind"][] = ["brush", "linear", "radial", "luminance"];
/** How a part's mode reads in front of its name, for the parts after the first. */
const MODE_MARKS: Record<MaskMode, string> = { add: "+", subtract: "−", intersect: "∩" };
/** Thumbnails are drawn this big along the frame's long side, for sharp 36-pixel squares on scaled screens. */
const MATTE_EDGE = 96;
/** The square a thumbnail sits in, in CSS pixels; `.matte` in styles.css. */
const MATTE_BOX = 36;

/**
 * Each mask's coverage as a black-and-white picture, redrawn a moment after
 * the recipe stops changing.
 */
function useMattes(): Map<number, ImageData> {
  const photoId = useStore((s) => s.editor.photoId);
  const ready = useStore((s) => s.editor.ready);
  const adjustments = useStore((s) => s.editor.adjustments);
  const [mattes, setMattes] = useState(new Map<number, ImageData>());
  useEffect(() => {
    if (!ready || photoId === null || adjustments.masks.length === 0) return;
    let current = true;
    const timer = setTimeout(async () => {
      try {
        const pictures = await api.maskMattes(photoId, adjustments, MATTE_EDGE);
        if (current) setMattes(new Map(adjustments.masks.map((mask, i) => [mask.id, pictures[i]])));
      } catch {
        // The photo was closed meanwhile; the next one draws its own.
      }
    }, 250);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [photoId, ready, adjustments]);
  return mattes;
}

function Matte({ pixels }: { pixels: ImageData | undefined }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useLayoutEffect(() => {
    const target = canvas.current;
    if (!target || !pixels) return;
    target.width = pixels.width;
    target.height = pixels.height;
    target.getContext("2d")!.putImageData(pixels, 0, 0);
  }, [pixels]);
  // The whole frame, fitted inside the square.
  const fit = pixels && MATTE_BOX / Math.max(pixels.width, pixels.height);
  return (
    <span className="matte">
      {pixels && fit && <canvas ref={canvas} style={{ width: pixels.width * fit, height: pixels.height * fit }} />}
    </span>
  );
}

function PartRow(props: { mask: Mask; part: MaskPart; index: number; selected: boolean }) {
  const { mask, part, index, selected } = props;
  const s = useStore.getState();
  const Icon = SHAPE_ICONS[part.shape.kind];
  const name = SHAPE_NAMES[part.shape.kind];
  const modes: MenuEntry[] = (["add", "subtract", "intersect"] as MaskMode[]).map((mode) => ({
    label: MODE_NAMES[mode],
    checked: part.mode === mode,
    run: () => s.updateMaskPart(index, { ...part, mode }, `${mask.name}: ${MODE_NAMES[mode].toLowerCase()} ${name.toLowerCase()}`),
  }));
  return (
    <li className={`layer-part ${selected ? "selected" : ""}`}>
      {/* The first part starts the mask; the rest add to it, take from it or narrow it. */}
      {index > 0 ? (
        <button className="mode-mark" title={`${MODE_NAMES[part.mode]}: change how this part combines`} onClick={(event) => menuBelow(event, modes)}>
          {MODE_MARKS[part.mode]}
        </button>
      ) : (
        <span className="mode-mark" />
      )}
      <button className="layer-part-main" aria-pressed={selected} onClick={() => s.selectMask(mask.id, index)}>
        <Icon size={13} />
        <span className="layer-name">{name}</span>
      </button>
      <button className="icon-button small" title={`Remove ${name.toLowerCase()}`} aria-label={`Remove ${name}`} onClick={() => s.removeMaskPart(index)}>
        <X size={13} />
      </button>
    </li>
  );
}

function Layer(props: { mask: Mask; matte: ImageData | undefined; selected: boolean; partIndex: number | null }) {
  const { mask, matte, selected, partIndex } = props;
  const masks = useStore((s) => s.editor.adjustments.masks);
  const [renaming, setRenaming] = useState(false);
  const s = useStore.getState();

  const entries: MenuEntry[] = [
    { label: "Rename", run: () => setRenaming(true) },
    { label: "Invert", checked: mask.invert, run: () => s.updateMask(mask.id, { invert: !mask.invert }, `${mask.name}: invert`) },
    "separator",
    { label: "Delete mask", danger: true, run: () => s.removeMask(mask.id) },
  ];
  const partMenu = (mode: MaskMode) => (event: React.MouseEvent) =>
    menuBelow(
      event,
      KINDS.map((kind) => ({
        label: SHAPE_NAMES[kind],
        disabled: !canAdd(masks, kind),
        run: () => {
          if (!selected) s.selectMask(mask.id);
          s.addMaskPart(kind, mode);
        },
      })),
    );

  return (
    <li className={`layer ${selected ? "selected" : ""} ${mask.visible ? "" : "hidden"}`}>
      <div className="layer-row">
        {renaming ? (
          <NameInput
            name={mask.name}
            label="Mask name"
            onDone={(name) => {
              setRenaming(false);
              const tidied = name.trim();
              if (tidied && tidied !== mask.name) s.updateMask(mask.id, { name: tidied }, `Rename ${mask.name} to ${tidied}`);
            }}
          />
        ) : (
          <button
            className="layer-main"
            aria-pressed={selected}
            onClick={() => s.selectMask(mask.id)}
            onDoubleClick={() => setRenaming(true)}
            onContextMenu={(event) => {
              event.preventDefault();
              s.openMenu(event.clientX, event.clientY, entries);
            }}
          >
            <Matte pixels={matte} />
            <span className="layer-name">{mask.name}</span>
          </button>
        )}
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
      </div>
      {selected && (
        <>
          <ul className="layer-parts">
            {mask.parts.map((part, index) => (
              <PartRow key={index} mask={mask} part={part} index={index} selected={index === partIndex} />
            ))}
          </ul>
          <div className="layer-actions">
            <button className="button quiet" onClick={partMenu("add")}>
              Add
            </button>
            <button className="button quiet" onClick={partMenu("subtract")}>
              Subtract
            </button>
            <button className="button quiet" onClick={partMenu("intersect")}>
              Intersect
            </button>
          </div>
        </>
      )}
    </li>
  );
}

/**
 * The photo's masks as layers, floating over the corner of the photo while
 * the Masks tool is open. Each shows what it covers; the chosen one opens
 * to show its parts.
 */
export function MaskLayers() {
  const masks = useStore((s) => s.editor.adjustments.masks);
  const maskId = useStore((s) => s.editor.maskId);
  const partIndex = useStore((s) => s.editor.partIndex);
  const showMask = useStore((s) => s.editor.showMask);
  const mattes = useMattes();
  const [open, setOpen] = useState(true);
  const s = useStore.getState();
  if (masks.length === 0) return null;

  return (
    <section className="mask-layers" aria-label="Masks">
      <header>
        <button className="layers-title" aria-expanded={open} onClick={() => setOpen(!open)}>
          <ChevronDown size={14} />
          Masks
        </button>
        <button className="icon-button small" title="New mask" aria-label="New mask" onClick={() => s.selectMask(null)}>
          <Plus size={15} />
        </button>
      </header>
      {open && (
        <>
          <ul className="layer-list">
            {masks.map((mask) => (
              <Layer key={mask.id} mask={mask} matte={mattes.get(mask.id)} selected={mask.id === maskId} partIndex={partIndex} />
            ))}
          </ul>
          <label className="toggle overlay-toggle">
            <input type="checkbox" checked={showMask} onChange={s.toggleMaskOverlay} />
            Show overlay
            <span className="key">O</span>
          </label>
        </>
      )}
    </section>
  );
}
