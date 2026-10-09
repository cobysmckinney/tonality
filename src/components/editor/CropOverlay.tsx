import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { Region } from "../../api";
import { dragged, Handle, heldAspect, onCanvas, Rect, Size, toCrop, toFrame, toRect, turnedSize } from "../../crop";
import { arrowOffset } from "../../nudge";
import { useStore } from "../../store";

const HANDLES: (Handle & { cursor: string })[] = [
  { x: -1, y: -1, cursor: "nwse-resize" },
  { x: 0, y: -1, cursor: "ns-resize" },
  { x: 1, y: -1, cursor: "nesw-resize" },
  { x: -1, y: 0, cursor: "ew-resize" },
  { x: 1, y: 0, cursor: "ew-resize" },
  { x: -1, y: 1, cursor: "nesw-resize" },
  { x: 0, y: 1, cursor: "ns-resize" },
  { x: 1, y: 1, cursor: "nwse-resize" },
];
/** What each corner is called, for a screen reader. */
const CORNERS: Record<string, string> = { "-1,-1": "Top left", "1,-1": "Top right", "-1,1": "Bottom left", "1,1": "Bottom right" };
/** How long after the last arrow key a run of nudges becomes one step, as with a slider. */
const NUDGE_PAUSE = 600;
/** How far around a corner or edge a press still counts as grabbing it, in screen pixels. */
const GRIP = 14;
const BRACKET = 18;

interface Props {
  /** The photo's full size in pixels. */
  photo: Size;
  /** The size of the whole tilted photo in photo pixels, and the part of it on screen (0..1 across and down). */
  frame: Size;
  region: Region;
  /** The size on screen of the canvas this lies over. */
  width: number;
  height: number;
}

/**
 * The crop frame drawn over the whole (tilted) photo. The frame always sits
 * upright; straightening turns the photo underneath it. Drag a corner or an
 * edge to resize, drag inside to move. From the keyboard the frame and its
 * corners take the focus, and the arrow keys move them.
 */
export function CropOverlay({ photo, frame, region, width, height }: Props) {
  const adjustments = useStore((s) => s.editor.adjustments);
  const chosen = useStore((s) => s.cropAspect);
  const drag = useRef<{ handle: Handle; start: Rect; aspect: number | null; x: number; y: number } | null>(null);
  // A run of arrow-key nudges, from where it started, until it is recorded.
  const nudging = useRef<{ start: Rect; aspect: number | null } | null>(null);
  const pause = useRef<ReturnType<typeof setTimeout>>(undefined);
  // Guides appear only while they help: thirds while the frame moves, a finer grid while straightening.
  const [dragging, setDragging] = useState(false);
  const [straightening, setStraightening] = useState(false);
  const firstAngle = useRef(adjustments.straighten);
  useEffect(() => {
    if (adjustments.straighten === firstAngle.current) return;
    firstAngle.current = NaN;
    setStraightening(true);
    const timer = setTimeout(() => setStraightening(false), 700);
    return () => clearTimeout(timer);
  }, [adjustments.straighten]);

  const turned = turnedSize(photo, adjustments.rotation);
  const rect = toRect(adjustments.crop, turned);
  // The chosen shape only holds while the crop still has it: undo can put back a crop of another shape.
  const aspect = heldAspect(rect, chosen);
  // Where the crop sits on screen: its offset from the photo's centre, as seen in the tilted view.
  const [offsetX, offsetY] = toFrame([rect.x - turned.width / 2, rect.y - turned.height / 2], adjustments.straighten);
  // Zoomed in, the photo's centre may be anywhere, even off the canvas.
  const centre = onCanvas(frame, region, { width, height });
  const scale = centre.scale;
  const w = rect.width * scale;
  const h = rect.height * scale;
  const left = centre.x + offsetX * scale - w / 2;
  const top = centre.y + offsetY * scale - h / 2;

  const begin = (handle: Handle) => (event: React.PointerEvent<SVGElement>) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    drag.current = { handle, start: rect, aspect, x: event.clientX, y: event.clientY };
    setDragging(true);
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const move = (event: React.PointerEvent) => {
    const held = drag.current;
    if (!held) return;
    // The button let go somewhere the frame never heard it: hovering alone never moves the crop.
    if ((event.buttons & 1) === 0) return end();
    const delta: [number, number] = [(event.clientX - held.x) / scale, (event.clientY - held.y) / scale];
    const next = dragged(held.start, held.handle, delta, adjustments.straighten, turned, held.aspect);
    useStore.getState().adjust({ crop: toCrop(next, turned) });
  };
  /** Records a drag or a run of nudges from `start` as one step, for undo in the crop tool. */
  const record = (start: Rect, held: number | null) => {
    const s = useStore.getState();
    // A shape that no longer held is let go once the crop is changed freely.
    const now = toRect(s.editor.adjustments.crop, turned);
    const moved = (["x", "y", "width", "height"] as const).some((key) => start[key] !== now[key]);
    if (held === null && s.cropAspect !== null && moved) s.setCropShape("Free", null);
    s.markCrop();
  };
  const end = () => {
    const held = drag.current;
    if (!held) return;
    drag.current = null;
    setDragging(false);
    record(held.start, held.aspect);
  };
  const endNudges = () => {
    clearTimeout(pause.current);
    const held = nudging.current;
    nudging.current = null;
    if (held) record(held.start, held.aspect);
  };
  // Ending up out of the crop tool mid-drag still ends the drag: leaving keeps it, Esc cancels it, and holding \ makes it a step to undo.
  const ending = useRef(end);
  ending.current = end;
  const endingNudges = useRef(endNudges);
  endingNudges.current = endNudges;
  useLayoutEffect(
    () => () => {
      ending.current();
      endingNudges.current();
    },
    [],
  );

  // An arrow key moves the frame, or a corner, a pixel on screen (ten with Shift), as a short drag would.
  const nudge = (handle: Handle) => (event: React.KeyboardEvent<SVGElement>) => {
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    // Delete here shouldn't fall through to deleting the photo.
    if (event.key === "Delete" || event.key === "Backspace") return event.stopPropagation();
    const offset = arrowOffset(event.key, event.shiftKey);
    if (!offset) return;
    // The arrows would otherwise also change photo.
    event.preventDefault();
    event.stopPropagation();
    nudging.current ??= { start: rect, aspect };
    const next = dragged(rect, handle, [offset[0] / scale, offset[1] / scale], adjustments.straighten, turned, nudging.current.aspect);
    useStore.getState().adjust({ crop: toCrop(next, turned) });
    clearTimeout(pause.current);
    pause.current = setTimeout(endNudges, NUDGE_PAUSE);
  };

  const grid = (parts: number) =>
    Array.from({ length: parts - 1 }, (_, i) => {
      const n = i + 1;
      return `M${left + (w * n) / parts} ${top}v${h}M${left} ${top + (h * n) / parts}h${w}`;
    }).join("");
  // Corner brackets, so the corners read as grabbable.
  const b = Math.min(BRACKET, w / 3, h / 3);
  const brackets =
    `M${left} ${top + b}v${-b}h${b}M${left + w - b} ${top}h${b}v${b}` +
    `M${left + w} ${top + h - b}v${b}h${-b}M${left + b} ${top + h}h${-b}v${-b}`;

  return (
    <svg
      className="crop-overlay"
      width={width}
      height={height}
      onPointerMove={move}
      onPointerUp={end}
      onPointerCancel={end}
    >
      <path className="crop-shade" fillRule="evenodd" d={`M0 0H${width}V${height}H0ZM${left} ${top}h${w}v${h}h${-w}Z`} />
      <rect
        className="crop-area"
        x={left}
        y={top}
        width={w}
        height={h}
        tabIndex={0}
        role="button"
        aria-label="Crop frame: arrow keys move it"
        onPointerDown={begin({ x: 0, y: 0 })}
        onKeyDown={nudge({ x: 0, y: 0 })}
        onBlur={endNudges}
      />
      {straightening ? (
        <path className="crop-thirds fine" d={grid(8)} />
      ) : (
        dragging && <path className="crop-thirds" d={grid(3)} />
      )}
      <rect className="crop-edge" x={left} y={top} width={w} height={h} />
      <path className="crop-brackets" d={brackets} />
      {HANDLES.map((handle) => {
        // Corners get a square grip; edges a strip between the corners.
        const gx = handle.x === 0 ? left + GRIP : handle.x < 0 ? left - GRIP : left + w - GRIP;
        const gy = handle.y === 0 ? top + GRIP : handle.y < 0 ? top - GRIP : top + h - GRIP;
        const gw = handle.x === 0 ? Math.max(0, w - 2 * GRIP) : 2 * GRIP;
        const gh = handle.y === 0 ? Math.max(0, h - 2 * GRIP) : 2 * GRIP;
        // The corners take the focus too; between them they do what the edges do.
        const corner = CORNERS[`${handle.x},${handle.y}`];
        return (
          <rect
            key={`${handle.x},${handle.y}`}
            className="crop-grip"
            x={gx}
            y={gy}
            width={gw}
            height={gh}
            style={{ cursor: handle.cursor }}
            onPointerDown={begin(handle)}
            {...(corner && {
              tabIndex: 0,
              role: "button",
              "aria-label": `${corner} corner: arrow keys move it`,
              onKeyDown: nudge(handle),
              onBlur: endNudges,
            })}
          />
        );
      })}
    </svg>
  );
}
