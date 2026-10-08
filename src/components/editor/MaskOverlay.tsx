import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { Adjustments, Mask, MaskPart, Point, Stroke } from "../../adjustments";
import { Size } from "../../crop";
import { brushRadius, ScreenMap, screenMap, SHAPE_NAMES, stillHeld, tidy } from "../../masks";
import { useStore } from "../../store";

interface Props {
  /** The photo's full size in pixels. */
  photo: Size;
  /** The part of the cropped picture on screen, 0..1. */
  region: { x: number; y: number; width: number; height: number };
  /** The size on screen of the canvas this lies over. */
  width: number;
  height: number;
}

/** How far from a handle a press still takes hold of it, in screen pixels. */
const GRIP = 10;
/** How far past a radial gradient's edge its turning knob sits. */
const KNOB_REACH = 26;
/** Long enough to cross any screen: the lines of a linear gradient. */
const ACROSS = 6000;

type Drag = (photo: Point, pointer: Point) => MaskPart | null;

/**
 * The parts of the mask being worked on, drawn over the photo with handles
 * to move and shape them. When the part chosen in the Masks panel is a
 * brush, dragging on the photo paints instead (Alt erases).
 */
export function MaskOverlay({ photo, region, width, height }: Props) {
  const adjustments = useStore((s) => s.editor.adjustments);
  const maskId = useStore((s) => s.editor.maskId);
  const partIndex = useStore((s) => s.editor.partIndex);
  const brush = useStore((s) => s.brush);
  const circling = useStore((s) => s.editor.circling);
  const svg = useRef<SVGSVGElement>(null);
  // A drag keeps the mask it started on and its step's name, so it can be ended after that mask is gone from view.
  const drag = useRef<{ maskId: number; index: number; move: Drag; label: string } | null>(null);
  const painting = useRef<{ maskId: number; index: number; erase: boolean; last: Point; label: string } | null>(null);
  const [pointer, setPointer] = useState<{ at: Point; alt: boolean } | null>(null);

  // Only the framing matters here; a brush stroke shouldn't rebuild the map.
  const { crop, straighten, rotation, flipHorizontal, flipVertical } = adjustments;
  const map = useMemo(
    () =>
      screenMap({
        photo,
        adjustments: { crop, straighten, rotation, flipHorizontal, flipVertical } as Adjustments,
        region,
        shown: { width, height },
      }),
    [photo, crop, straighten, rotation, flipHorizontal, flipVertical, region, width, height],
  );

  /** Ends a drag or a brush stroke, recording what it changed as one step. */
  const finish = () => {
    const held = drag.current ?? painting.current;
    drag.current = null;
    painting.current = null;
    if (held) useStore.getState().commitAdjust(held.label);
  };
  // Choosing another mask or part, drawing a loop or closing the tool takes the handles away: the drag ends there,
  // rather than carrying on into whatever is chosen next. Layout effects, so no pointer event slips in first.
  useLayoutEffect(() => {
    const held = drag.current ?? painting.current;
    if (held && !stillHeld({ maskId: held.maskId, partIndex: held.index }, { maskId, partIndex, circling: circling !== null })) finish();
  }, [maskId, partIndex, circling]);
  useLayoutEffect(() => () => finish(), []);

  if (circling) return <CircleDrawing map={map} width={width} height={height} finished={circling.points} />;
  const mask = adjustments.masks.find((m) => m.id === maskId);
  if (!mask) return null;
  const selected = partIndex !== null ? mask.parts[partIndex] : undefined;
  const brushing = selected?.shape.kind === "brush";

  const local = (event: React.PointerEvent): Point => {
    const box = svg.current!.getBoundingClientRect();
    return [event.clientX - box.left, event.clientY - box.top];
  };

  const begin = (index: number, move: Drag) => (event: React.PointerEvent<SVGElement>) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    const part = mask.parts[index];
    drag.current = { maskId: mask.id, index, move, label: `${mask.name}: ${SHAPE_NAMES[part.shape.kind].toLowerCase()}` };
    if (index !== partIndex) useStore.getState().selectMask(mask.id, index);
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const radius = brushRadius(brush.size) * map.scale;
  const paintStart = (event: React.PointerEvent<SVGElement>) => {
    if (event.button !== 0 || partIndex === null || !brushing) return;
    event.stopPropagation();
    const at = local(event);
    const erase = brush.erase !== event.altKey;
    painting.current = { maskId: mask.id, index: partIndex, erase, last: at, label: `${mask.name}: ${erase ? "erase" : "brush"}` };
    event.currentTarget.setPointerCapture(event.pointerId);
    const stroke = {
      points: [map.toPhoto(at).map(tidy) as Point],
      radius: tidy(brushRadius(brush.size)),
      feather: brush.feather / 100,
      strength: brush.strength / 100,
      erase,
    };
    withBrush(partIndex, (strokes) => [...strokes, stroke]);
  };

  const onMove = (event: React.PointerEvent) => {
    const at = local(event);
    if (brushing) setPointer({ at, alt: event.altKey });
    // The button let go somewhere the overlay never heard it: hovering alone never changes anything.
    if ((event.buttons & 1) === 0) finish();
    const held = drag.current;
    if (held) {
      const part = held.move(map.toPhoto(at), at);
      if (part) useStore.getState().updateMaskPart(held.index, part);
      return;
    }
    const stroke = painting.current;
    // A point every so often along the way is plenty: the stroke is drawn as straight runs between them.
    if (stroke && Math.hypot(at[0] - stroke.last[0], at[1] - stroke.last[1]) >= Math.max(2, radius * 0.12)) {
      stroke.last = at;
      const point = map.toPhoto(at).map(tidy) as Point;
      withBrush(stroke.index, (strokes) => {
        const last = strokes[strokes.length - 1];
        return [...strokes.slice(0, -1), { ...last, points: [...last.points, point] }];
      });
    }
  };

  return (
    <svg
      ref={svg}
      className={`mask-overlay ${brushing ? "brushing" : ""}`}
      width={width}
      height={height}
      onPointerMove={onMove}
      onPointerUp={finish}
      onPointerCancel={finish}
      onPointerLeave={() => setPointer(null)}
    >
      {brushing && <rect className="mask-paint" width={width} height={height} onPointerDown={paintStart} />}
      {mask.parts.map((part, index) => (
        <PartHandles
          key={index}
          part={part}
          map={map}
          active={index === partIndex}
          begin={(move) => begin(index, move)}
        />
      ))}
      {brushing && pointer && (
        <g className={`brush-cursor ${brush.erase !== pointer.alt ? "erasing" : ""}`}>
          <circle cx={pointer.at[0]} cy={pointer.at[1]} r={radius} />
          {brush.feather > 0 && <circle className="inner" cx={pointer.at[0]} cy={pointer.at[1]} r={radius * (1 - brush.feather / 100)} />}
        </g>
      )}
    </svg>
  );
}

/** An outline through screen points, closed. */
const outline = (points: Point[]) => (points.length ? `M${points.map((p) => `${p[0]} ${p[1]}`).join("L")}Z` : "");

/**
 * Drawing a loop around something on the photo, for its object to be found.
 * Once `finished` (points on the file), the loop stays drawn while the
 * object is found.
 */
function CircleDrawing({ map, width, height, finished }: { map: ScreenMap; width: number; height: number; finished?: Point[] }) {
  // The points as drawn, kept apart from the render so the last moves are never missed.
  const points = useRef<Point[] | null>(null);
  const [drawn, setDrawn] = useState<Point[]>([]);
  const local = (event: React.PointerEvent<SVGElement>): Point => {
    const box = event.currentTarget.closest("svg")!.getBoundingClientRect();
    return [event.clientX - box.left, event.clientY - box.top];
  };
  const shown = finished ? finished.map((p) => map.toScreen(p)) : drawn;
  return (
    <svg className={`mask-overlay circling ${finished ? "finding" : ""}`} width={width} height={height}>
      <rect
        className="mask-paint"
        width={width}
        height={height}
        onPointerDown={(event) => {
          if (event.button !== 0 || finished) return;
          event.stopPropagation();
          event.currentTarget.setPointerCapture(event.pointerId);
          points.current = [local(event)];
          setDrawn(points.current);
        }}
        onPointerMove={(event) => {
          const loop = points.current;
          if (!loop) return;
          const at = local(event);
          const last = loop[loop.length - 1];
          if (Math.hypot(at[0] - last[0], at[1] - last[1]) < 3) return;
          points.current = [...loop, at];
          setDrawn(points.current);
        }}
        onPointerUp={() => {
          const loop = points.current;
          if (!loop) return;
          points.current = null;
          // A dab, or a stroke too small to hold anything, is let go.
          const xs = loop.map((p) => p[0]);
          const ys = loop.map((p) => p[1]);
          if (loop.length < 3 || Math.max(Math.max(...xs) - Math.min(...xs), Math.max(...ys) - Math.min(...ys)) < 12) {
            setDrawn([]);
            return;
          }
          useStore.getState().finishCircle(loop.map((p) => map.toPhoto(p)));
        }}
      />
      {shown.length > 1 && (
        <>
          <path className="circle-shade" d={`M0 0H${width}V${height}H0Z${outline(shown)}`} fillRule="evenodd" />
          <path className={`mask-line ${finished ? "dashed" : ""}`} d={finished ? outline(shown) : outline(shown).slice(0, -1)} />
        </>
      )}
    </svg>
  );
}

/** Changes the strokes of a brush part of the mask being worked on, live. */
function withBrush(index: number, change: (strokes: Stroke[]) => Stroke[]) {
  const state = useStore.getState();
  const mask: Mask | undefined = state.editor.adjustments.masks.find((m) => m.id === state.editor.maskId);
  const part = mask?.parts[index];
  if (!part || part.shape.kind !== "brush") return;
  const strokes = change(part.shape.strokes);
  state.updateMaskPart(index, { ...part, shape: { kind: "brush", strokes } });
}

const add = (a: Point, b: Point): Point => [a[0] + b[0], a[1] + b[1]];
const sub = (a: Point, b: Point): Point => [a[0] - b[0], a[1] - b[1]];
const times = (a: Point, k: number): Point => [a[0] * k, a[1] * k];
const length = (a: Point) => Math.hypot(a[0], a[1]);

function PartHandles(props: { part: MaskPart; map: ScreenMap; active: boolean; begin: (move: Drag) => (event: React.PointerEvent<SVGElement>) => void }) {
  const { part, map, active, begin } = props;
  const { shape } = part;
  const className = `mask-part ${active ? "active" : ""} ${part.mode === "add" ? "" : part.mode}`;

  if (shape.kind === "linear") {
    const from = map.toScreen(shape.from);
    const to = map.toScreen(shape.to);
    const along = sub(to, from);
    const span = length(along) || 1;
    const across = times([-along[1] / span, along[0] / span], ACROSS);
    const line = (at: Point) => `M${at[0] - across[0]} ${at[1] - across[1]}L${at[0] + across[0]} ${at[1] + across[1]}`;
    const middle = times(add(from, to), 0.5);
    // Moving the middle carries both ends along by the same amount on the photo.
    const moveBoth: Drag = (() => {
      let start: { pointer: Point; from: Point; to: Point } | null = null;
      return (photo) => {
        start ??= { pointer: photo, from: shape.from, to: shape.to };
        const delta = sub(photo, start.pointer);
        return { ...part, shape: { ...shape, from: add(start.from, delta).map(tidy) as Point, to: add(start.to, delta).map(tidy) as Point } };
      };
    })();
    return (
      <g className={className}>
        <path className="mask-line" d={line(from)} />
        <path className="mask-line faint" d={line(to)} />
        <path className="mask-line dashed" d={line(middle)} />
        <path className="mask-line grip" d={`M${from[0]} ${from[1]}L${to[0]} ${to[1]}`} />
        <Knob at={from} onPointerDown={begin((photo) => ({ ...part, shape: { ...shape, from: photo.map(tidy) as Point } }))} />
        <Knob at={to} onPointerDown={begin((photo) => ({ ...part, shape: { ...shape, to: photo.map(tidy) as Point } }))} />
        <Knob at={middle} pin onPointerDown={begin(moveBoth)} />
      </g>
    );
  }

  if (shape.kind === "radial") {
    const center = map.toScreen(shape.center);
    const angle = (shape.angle * Math.PI) / 180;
    const [cos, sin] = [Math.cos(angle), Math.sin(angle)];
    // The ellipse's two radii, on screen, from the direction they have on the file.
    const first = map.direction([cos * shape.radius[0] * map.long, sin * shape.radius[0] * map.long]);
    const second = map.direction([-sin * shape.radius[1] * map.long, cos * shape.radius[1] * map.long]);
    const [rx, ry] = [length(first), length(second)];
    const turn = (Math.atan2(first[1], first[0]) * 180) / Math.PI;
    const inner = 1 - shape.feather;

    /** Where the pointer is on the file, in pixels, measured from the centre along the ellipse's own axes. */
    const own = (photo: Point): Point => {
      const offset = sub(map.toPixels(photo), map.toPixels(shape.center));
      return [offset[0] * cos + offset[1] * sin, -offset[0] * sin + offset[1] * cos];
    };
    const resize = (axis: 0 | 1): Drag => (photo) => {
      const reach = Math.max(0.003, tidy(Math.abs(own(photo)[axis]) / map.long));
      const radius: [number, number] = axis === 0 ? [reach, shape.radius[1]] : [shape.radius[0], reach];
      return { ...part, shape: { ...shape, radius } };
    };
    const rotate: Drag = (photo) => {
      const offset = sub(map.toPixels(photo), map.toPixels(shape.center));
      return { ...part, shape: { ...shape, angle: Math.round((Math.atan2(offset[1], offset[0]) * 1800) / Math.PI) / 10 } };
    };
    const moveCenter: Drag = (() => {
      let start: { pointer: Point; center: Point } | null = null;
      return (photo) => {
        start ??= { pointer: photo, center: shape.center };
        return { ...part, shape: { ...shape, center: add(start.center, sub(photo, start.pointer)).map(tidy) as Point } };
      };
    })();
    const knob = add(center, times(first, 1 + KNOB_REACH / Math.max(rx, 1)));

    return (
      <g className={className}>
        <ellipse className="mask-line" cx={center[0]} cy={center[1]} rx={rx} ry={ry} transform={`rotate(${turn} ${center[0]} ${center[1]})`} />
        {inner > 0.02 && (
          <ellipse
            className="mask-line dashed"
            cx={center[0]}
            cy={center[1]}
            rx={rx * inner}
            ry={ry * inner}
            transform={`rotate(${turn} ${center[0]} ${center[1]})`}
          />
        )}
        <path className="mask-line faint" d={`M${center[0]} ${center[1]}L${knob[0]} ${knob[1]}`} />
        <Knob at={add(center, first)} onPointerDown={begin(resize(0))} />
        <Knob at={sub(center, first)} onPointerDown={begin(resize(0))} />
        <Knob at={add(center, second)} onPointerDown={begin(resize(1))} />
        <Knob at={sub(center, second)} onPointerDown={begin(resize(1))} />
        <Knob at={knob} turn onPointerDown={begin(rotate)} />
        <Knob at={center} pin onPointerDown={begin(moveCenter)} />
      </g>
    );
  }

  if (shape.kind === "object") {
    // The loop it was found in; drawing another is in the Masks panel.
    return (
      <g className={className}>
        <path className="mask-line dashed" d={outline(shape.points.map((p) => map.toScreen(p)))} />
      </g>
    );
  }
  return null;
}

function Knob(props: { at: Point; pin?: boolean; turn?: boolean; onPointerDown: (event: React.PointerEvent<SVGElement>) => void }) {
  const { at, pin, turn, onPointerDown } = props;
  return (
    <g className={`mask-knob ${pin ? "pin" : ""} ${turn ? "turn" : ""}`} onPointerDown={onPointerDown}>
      <circle className="mask-knob-grip" cx={at[0]} cy={at[1]} r={GRIP} />
      <circle className="mask-knob-dot" cx={at[0]} cy={at[1]} r={pin ? 6 : 4.5} />
    </g>
  );
}
