import { useEffect, useLayoutEffect, useRef, useState } from "react";
import {
  CircleDashed,
  Blend,
  Cloud,
  Ellipsis,
  Eye,
  EyeOff,
  FlipHorizontal2,
  LassoSelect,
  LucideIcon,
  Paintbrush,
  Plus,
  Rows3,
  SquaresIntersect,
  SquaresSubtract,
  SquaresUnite,
  SunMedium,
  User,
  X,
} from "lucide-react";
import { Adjustments, Mask, MaskMode, MaskPart, Shape } from "../../adjustments";
import { api } from "../../api";
import { canAdd, matteKey, MODE_NAMES, SHAPE_NAMES } from "../../masks";
import { MenuEntry, useStore } from "../../store";
import { menuBelow } from "../Toolbar";
import { NameInput } from "./NameInput";

export const SHAPE_ICONS = {
  brush: Paintbrush,
  linear: Rows3,
  radial: CircleDashed,
  luminance: SunMedium,
  subject: User,
  sky: Cloud,
  object: LassoSelect,
};

const KINDS: Shape["kind"][] = ["subject", "sky", "object", "brush", "linear", "radial", "luminance"];
/** How each way of combining a part is drawn, in front of the parts after the first. */
export const MODE_ICONS: Record<MaskMode, LucideIcon> = { add: SquaresUnite, subtract: SquaresSubtract, intersect: SquaresIntersect };
/** Thumbnails are drawn this big along the frame's long side, for sharp squares on scaled screens. */
const MATTE_EDGE = 96;
/** The square a thumbnail sits in, in CSS pixels; `.matte` in styles.css. */
const MATTE_BOX = 32;

/** Each recipe's `matteKey`, worked out once: the store asks on every change to anything. */
const matteKeys = new WeakMap<Adjustments, string>();
function matteKeyOf(adjustments: Adjustments): string {
  let key = matteKeys.get(adjustments);
  if (key === undefined) {
    key = matteKey(adjustments);
    matteKeys.set(adjustments, key);
  }
  return key;
}

/**
 * Each mask's coverage as a black-and-white picture, redrawn a moment after
 * something that can change one stops changing.
 */
function useMattes(): Map<number, ImageData> {
  const photoId = useStore((s) => s.editor.photoId);
  const ready = useStore((s) => s.editor.ready);
  // A slider that can't change a mask leaves the key, and the pictures, alone.
  const key = useStore((s) => matteKeyOf(s.editor.adjustments));
  // Parts found again in a new crop change the pictures too.
  const found = useStore((s) => s.editor.found);
  // Kept with the photo they were drawn for: another photo's masks can have the same ids.
  const [mattes, setMattes] = useState({ photoId, pictures: new Map<number, ImageData>() });
  useEffect(() => {
    const { adjustments } = useStore.getState().editor;
    if (!ready || photoId === null || adjustments.masks.length === 0) return;
    let current = true;
    const timer = setTimeout(async () => {
      try {
        const pictures = await api.maskMattes(photoId, adjustments, MATTE_EDGE);
        if (current) setMattes({ photoId, pictures: new Map(adjustments.masks.map((mask, i) => [mask.id, pictures[i]])) });
      } catch {
        // The photo was closed meanwhile; the next one draws its own.
      }
    }, 250);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [photoId, ready, key, found]);
  return mattes.photoId === photoId ? mattes.pictures : NO_MATTES;
}

const NO_MATTES = new Map<number, ImageData>();

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
  return <span className="matte">{pixels && fit && <canvas ref={canvas} style={{ width: pixels.width * fit, height: pixels.height * fit }} />}</span>;
}

function PartRow(props: { mask: Mask; part: MaskPart; index: number; selected: boolean }) {
  const { mask, part, index, selected } = props;
  const s = useStore.getState();
  const Icon = SHAPE_ICONS[part.shape.kind];
  const Mode = MODE_ICONS[part.mode];
  const name = SHAPE_NAMES[part.shape.kind];
  const modes: MenuEntry[] = (["add", "subtract", "intersect"] as MaskMode[]).map((mode) => ({
    label: MODE_NAMES[mode],
    checked: part.mode === mode,
    run: () => s.updateMaskPart(index, { ...part, mode }, `${mask.name}: ${MODE_NAMES[mode].toLowerCase()} ${name.toLowerCase()}`),
  }));
  return (
    <li className={`mask-part-row ${selected ? "selected" : ""}`}>
      <button className="mask-part-main" aria-pressed={selected} onClick={() => s.selectMask(mask.id, index)}>
        <Icon size={14} strokeWidth={1.75} />
        <span className="layer-name">{name}</span>
      </button>
      {/* The first part starts the mask; the rest add to it, take from it or narrow it. */}
      {index > 0 && (
        <button
          className="icon-button small mode-mark"
          title={`${MODE_NAMES[part.mode]}: change how this part combines`}
          aria-label={`${name}: ${MODE_NAMES[part.mode]}`}
          onClick={(event) => menuBelow(event, modes)}
        >
          <Mode size={14} strokeWidth={1.75} />
        </button>
      )}
      <button className="icon-button small reveal" title={`Remove ${name.toLowerCase()}`} aria-label={`Remove ${name}`} onClick={() => s.removeMaskPart(index)}>
        <X size={13} />
      </button>
    </li>
  );
}

/** Under the chosen mask: its parts, and the buttons that add to it or turn it inside out. */
function MaskParts({ mask, partIndex }: { mask: Mask; partIndex: number | null }) {
  const masks = useStore((s) => s.editor.adjustments.masks);
  const finding = useStore((s) => s.editor.finding !== null);
  const circling = useStore((s) => s.editor.circling !== null && !s.editor.circling.points);
  const s = useStore.getState();
  const partMenu = (mode: MaskMode) => (event: React.MouseEvent) =>
    menuBelow(
      event,
      KINDS.map((kind) => ({ label: SHAPE_NAMES[kind], disabled: !canAdd(masks, kind), run: () => s.addMaskPart(kind, mode) })),
    );
  return (
    <div className="mask-parts">
      <ul>
        {mask.parts.map((part, index) => (
          <PartRow key={index} mask={mask} part={part} index={index} selected={index === partIndex} />
        ))}
      </ul>
      {circling ? (
        <div className="mask-part-actions">
          <span className="layer-note">Draw a loop on the photo</span>
          <button className="button quiet" onClick={() => s.cancelCircle()}>
            Cancel
          </button>
        </div>
      ) : (
        <div className="mask-part-actions" aria-busy={finding}>
          {(["add", "subtract", "intersect"] as MaskMode[]).map((mode) => (
            <button key={mode} className="button quiet" disabled={finding} title={`${MODE_NAMES[mode]} a part`} onClick={partMenu(mode)}>
              {MODE_NAMES[mode]}
            </button>
          ))}
          {finding && <span className="spinner" />}
          <button
            className={`icon-button small invert ${mask.invert ? "engaged" : ""}`}
            aria-pressed={mask.invert}
            aria-label="Invert"
            title={mask.invert ? "Inverted: changes everything but this" : "Invert: change everything but this"}
            onClick={() => s.updateMask(mask.id, { invert: !mask.invert }, `${mask.name}: invert`)}
          >
            <FlipHorizontal2 size={14} strokeWidth={1.75} />
          </button>
        </div>
      )}
    </div>
  );
}

function MaskRow(props: { mask: Mask; matte: ImageData | undefined; selected: boolean; partIndex: number | null }) {
  const { mask, matte, selected, partIndex } = props;
  const [renaming, setRenaming] = useState(false);
  // The part that was chosen when a click let go of this mask, so a double-click can choose it again.
  const lastPart = useRef<number | null>(null);
  const s = useStore.getState();

  const entries: MenuEntry[] = [
    { label: "Rename", run: () => setRenaming(true) },
    { label: "Invert", checked: mask.invert, run: () => s.updateMask(mask.id, { invert: !mask.invert }, `${mask.name}: invert`) },
    "separator",
    { label: "Delete mask", danger: true, run: () => s.removeMask(mask.id) },
  ];

  return (
    <li className={`mask-row ${selected ? "selected" : ""} ${mask.visible ? "" : "hidden"}`}>
      <div className="mask-row-head">
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
            className="mask-row-main"
            aria-pressed={selected}
            // A second click on the chosen mask lets go of it, back to starting a new one.
            // The second click of a double-click is the rename's, not another toggle.
            onClick={(event) => {
              if (event.detail > 1) return;
              lastPart.current = selected ? partIndex : null;
              s.selectMask(selected ? null : mask.id);
            }}
            // Renaming a mask chooses it, as it was if the first click let go of it.
            onDoubleClick={() => {
              if (!selected) s.selectMask(mask.id, lastPart.current ?? 0);
              setRenaming(true);
            }}
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
          className={`icon-button small ${mask.visible ? "reveal" : ""}`}
          title={mask.visible ? "Hide this mask’s effect" : "Show this mask’s effect"}
          aria-label={mask.visible ? `Hide ${mask.name}` : `Show ${mask.name}`}
          aria-pressed={!mask.visible}
          onClick={() => s.updateMask(mask.id, { visible: !mask.visible }, `${mask.visible ? "Hide" : "Show"} ${mask.name}`)}
        >
          {mask.visible ? <Eye size={14} /> : <EyeOff size={14} />}
        </button>
        <button
          className="icon-button small reveal"
          title="Rename, invert or delete"
          aria-label={`More for ${mask.name}`}
          onClick={(event) => menuBelow(event, entries)}
        >
          <Ellipsis size={14} />
        </button>
      </div>
      {selected && <MaskParts mask={mask} partIndex={partIndex} />}
    </li>
  );
}

/**
 * The photo's masks, each with a picture of what it covers. The chosen one
 * opens to show its parts; under the list, another mask is started and the
 * chosen one's tint on the photo turned on and off.
 */
export function MaskList() {
  const masks = useStore((s) => s.editor.adjustments.masks);
  const maskId = useStore((s) => s.editor.maskId);
  const partIndex = useStore((s) => s.editor.partIndex);
  const showMask = useStore((s) => s.editor.showMask);
  const mattes = useMattes();
  const s = useStore.getState();

  return (
    <section className="mask-list" aria-label="Masks">
      <ul>
        {masks.map((mask) => (
          <MaskRow key={mask.id} mask={mask} matte={mattes.get(mask.id)} selected={mask.id === maskId} partIndex={partIndex} />
        ))}
      </ul>
      {maskId !== null && (
        <div className="mask-list-footer">
          <button className="button quiet" onClick={() => s.selectMask(null)}>
            <Plus size={14} />
            New mask
          </button>
          <button
            className={`button quiet ${showMask ? "engaged" : ""}`}
            aria-pressed={showMask}
            title="Tint the chosen mask on the photo (O)"
            onClick={s.toggleMaskOverlay}
          >
            <Blend size={14} strokeWidth={1.75} />
            Overlay
          </button>
        </div>
      )}
    </section>
  );
}
