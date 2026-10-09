import { useLayoutEffect, useMemo, useRef, useState } from "react";
import { Adjustments, Point, Spot } from "../../adjustments";
import { Size } from "../../crop";
import { HEAL_STEPS, healRadius, movePatch, moveSpot, pathThrough, patchPoints, simplify, travel } from "../../heal";
import { screenMap } from "../../masks";
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

/** How far outside a spot's edge a press still takes hold of it, in screen pixels. */
const GRIP = 4;
/** A press that moves less than this far on screen is a click: a spot, not a line. */
const CLICK = 4;

/** A spot or its patch being dragged: which, from where (on the file), and the spot as it was. */
interface Drag {
  index: number;
  patch: boolean;
  from: Point;
  spot: Spot;
  moved: boolean;
}

/**
 * The Heal tool on the photo. Clicking covers a speck with a spot, dragging
 * paints a line over a hair or scratch; each takes its texture from a patch
 * found nearby. Spots can be chosen and dragged, and the chosen one's patch
 * too. Specks found as dust are marked, and clicking one heals it.
 */
export function HealOverlay({ photo, region, width, height }: Props) {
  const adjustments = useStore((s) => s.editor.adjustments);
  const spotIndex = useStore((s) => s.editor.spotIndex);
  const dust = useStore((s) => s.editor.dust);
  const size = useStore((s) => s.healSize);
  const svg = useRef<SVGSVGElement>(null);
  const drag = useRef<Drag | null>(null);
  // The line being drawn, on screen; kept apart from the render so no move is missed.
  const line = useRef<Point[] | null>(null);
  const [drawn, setDrawn] = useState<{ points: Point[]; radius: number } | null>(null);
  const [pointer, setPointer] = useState<Point | null>(null);

  // Only the framing matters here.
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

  /** Ends a drag or a line, recording what it did. */
  const finish = () => {
    const held = drag.current;
    drag.current = null;
    if (held?.moved) useStore.getState().commitAdjust(held.patch ? HEAL_STEPS.patch : HEAL_STEPS.moved);
    const points = line.current;
    line.current = null;
    if (!points) return;
    const radius = healRadius(useStore.getState().healSize) * map.scale;
    // A click is a spot where it was pressed; anything longer, a line with the points that shape it.
    const kept = travel(points) < CLICK ? [points[0]] : simplify(points, Math.max(1, radius * 0.2));
    setDrawn({ points: kept, radius });
    // The line stays drawn until its patch is found and the spot takes its place.
    void useStore
      .getState()
      .heal(kept.map((p) => map.toPhoto(p)))
      .finally(() => setDrawn(null));
  };
  useLayoutEffect(() => () => finish(), []);

  const local = (event: React.PointerEvent): Point => {
    const box = svg.current!.getBoundingClientRect();
    return [event.clientX - box.left, event.clientY - box.top];
  };

  const radius = healRadius(size) * map.scale;
  const draw = (event: React.PointerEvent<SVGElement>) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    const at = local(event);
    line.current = [at];
    setDrawn({ points: [at], radius });
    useStore.getState().selectSpot(null);
  };

  const hold = (index: number, patch: boolean) => (event: React.PointerEvent<SVGElement>) => {
    if (event.button !== 0) return;
    event.stopPropagation();
    event.currentTarget.setPointerCapture(event.pointerId);
    const spot = useStore.getState().editor.adjustments.spots[index];
    if (!spot) return;
    drag.current = { index, patch, from: map.toPhoto(local(event)), spot, moved: false };
    useStore.getState().selectSpot(index);
  };

  const onMove = (event: React.PointerEvent) => {
    const at = local(event);
    setPointer(at);
    // The button let go somewhere the overlay never heard it: hovering alone never changes anything.
    if ((event.buttons & 1) === 0) return finish();
    const held = drag.current;
    if (held) {
      const here = map.toPhoto(at);
      const by: Point = [here[0] - held.from[0], here[1] - held.from[1]];
      const screen = map.direction([by[0] * photo.width, by[1] * photo.height]);
      if (!held.moved && Math.hypot(screen[0], screen[1]) < 2) return;
      held.moved = true;
      useStore.getState().updateSpot(held.index, held.patch ? movePatch(held.spot, by) : moveSpot(held.spot, by));
      return;
    }
    const points = line.current;
    if (points) {
      const last = points[points.length - 1];
      if (Math.hypot(at[0] - last[0], at[1] - last[1]) < Math.max(2, radius * 0.25)) return;
      line.current = [...points, at];
      setDrawn({ points: line.current, radius });
    }
  };

  const spots = adjustments.spots;
  const chosen = spotIndex !== null ? spots[spotIndex] : undefined;
  const onScreen = (spot: Spot, points = spot.points) => ({ points: points.map((p) => map.toScreen(p)), radius: spot.radius * map.scale });
  return (
    <svg
      ref={svg}
      className="mask-overlay heal-overlay"
      width={width}
      height={height}
      onPointerMove={onMove}
      onPointerUp={finish}
      onPointerCancel={finish}
      onPointerLeave={() => setPointer(null)}
    >
      <rect className="mask-paint" width={width} height={height} onPointerDown={draw} />
      {spots.map((spot, index) => (
        <SpotArea
          key={index}
          {...onScreen(spot)}
          className={index === spotIndex ? "spot chosen" : "spot"}
          onPointerDown={hold(index, false)}
        />
      ))}
      {chosen && spotIndex !== null && (
        <>
          <path
            className="mask-line faint"
            d={pathThrough([map.toScreen(chosen.points[0]), map.toScreen(chosen.source)])}
          />
          <SpotArea {...onScreen(chosen, patchPoints(chosen))} className="spot patch" onPointerDown={hold(spotIndex, true)} />
        </>
      )}
      {dust?.map((speck, index) => {
        const [x, y] = map.toScreen(speck.center);
        return (
          <g
            key={`${speck.center[0]},${speck.center[1]}`}
            className="speck"
            role="button"
            aria-label="Speck of dust: click to remove it, Alt-click to leave it"
            onPointerDown={(event) => {
              if (event.button !== 0 && event.button !== 2) return;
              event.stopPropagation();
              const s = useStore.getState();
              if (event.altKey || event.button === 2) s.dismissSpeck(index);
              else s.removeSpecks([index]);
            }}
            onContextMenu={(event) => event.preventDefault()}
          >
            <circle className="speck-grip" cx={x} cy={y} r={Math.max(speck.radius * map.scale, 4) + GRIP + 2} />
            <circle className="speck-mark" cx={x} cy={y} r={Math.max(speck.radius * map.scale, 4) + 2} />
          </g>
        );
      })}
      {drawn && <path className="heal-stroke" d={pathThrough(drawn.points)} strokeWidth={drawn.radius * 2} />}
      {pointer && !drawn && !drag.current && <circle className="heal-cursor" cx={pointer[0]} cy={pointer[1]} r={radius} />}
    </svg>
  );
}

/**
 * A spot's area on screen: a circle round a spot, a faint band along a
 * line. A transparent grip a little wider takes presses.
 */
function SpotArea(props: {
  points: Point[];
  radius: number;
  className: string;
  onPointerDown: (event: React.PointerEvent<SVGElement>) => void;
}) {
  const { points, radius, className, onPointerDown } = props;
  if (points.length === 1) {
    const [x, y] = points[0];
    return (
      <g className={className}>
        <circle className="spot-grip" cx={x} cy={y} r={radius + GRIP} onPointerDown={onPointerDown} />
        <circle className="spot-edge" cx={x} cy={y} r={radius} />
      </g>
    );
  }
  // A faint band as wide as the line covers, and the line itself down its middle.
  const d = pathThrough(points);
  return (
    <g className={className}>
      <path d={d} className="spot-grip" strokeWidth={radius * 2 + GRIP * 2} onPointerDown={onPointerDown} />
      <path d={d} className="spot-band" strokeWidth={radius * 2} />
      <path d={d} className="spot-edge" />
    </g>
  );
}
