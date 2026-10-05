import { useEffect, useRef, useState } from "react";
import { dragged, Handle, Rect, Size, toCrop, toFrame, toRect, turnedSize } from "../../crop";
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
/** How far around a corner or edge a press still counts as grabbing it, in screen pixels. */
const GRIP = 14;
const BRACKET = 18;

interface Props {
  /** The photo's full size in pixels. */
  photo: Size;
  /** Screen pixels per photo pixel. */
  scale: number;
  /** The size on screen of the canvas this lies over. */
  width: number;
  height: number;
}

/**
 * The crop frame drawn over the whole (tilted) photo. The frame always sits
 * upright; straightening turns the photo underneath it. Drag a corner or an
 * edge to resize, drag inside to move.
 */
export function CropOverlay({ photo, scale, width, height }: Props) {
  const adjustments = useStore((s) => s.editor.adjustments);
  const aspect = useStore((s) => s.cropAspect);
  const drag = useRef<{ handle: Handle; start: Rect; x: number; y: number } | null>(null);
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
  // Where the crop sits on screen: its offset from the photo's centre, as seen in the tilted view.
  const [offsetX, offsetY] = toFrame([rect.x - turned.width / 2, rect.y - turned.height / 2], adjustments.straighten);
  const w = rect.width * scale;
  const h = rect.height * scale;
  const left = width / 2 + offsetX * scale - w / 2;
  const top = height / 2 + offsetY * scale - h / 2;

  const begin = (handle: Handle) => (event: React.PointerEvent<SVGElement>) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    drag.current = { handle, start: rect, x: event.clientX, y: event.clientY };
    setDragging(true);
    event.currentTarget.setPointerCapture(event.pointerId);
  };
  const move = (event: React.PointerEvent) => {
    const held = drag.current;
    if (!held) return;
    const delta: [number, number] = [(event.clientX - held.x) / scale, (event.clientY - held.y) / scale];
    const next = dragged(held.start, held.handle, delta, adjustments.straighten, turned, aspect);
    useStore.getState().adjust({ crop: toCrop(next, turned) });
  };
  const end = () => {
    if (!drag.current) return;
    drag.current = null;
    setDragging(false);
    useStore.getState().commitAdjust();
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
      onDoubleClick={(event) => event.stopPropagation()}
    >
      <path className="crop-shade" fillRule="evenodd" d={`M0 0H${width}V${height}H0ZM${left} ${top}h${w}v${h}h${-w}Z`} />
      <rect className="crop-area" x={left} y={top} width={w} height={h} onPointerDown={begin({ x: 0, y: 0 })} />
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
          />
        );
      })}
    </svg>
  );
}
