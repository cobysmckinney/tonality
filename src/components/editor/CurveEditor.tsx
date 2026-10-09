import { useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { CurveChannel, CurvePoints, curveSampler, isStraight } from "../../adjustments";
import { addPointNear, movePoint, nudgePoint, removePoint } from "../../curve";
import { useStore } from "../../store";
import { onTabKey } from "../../tabs";

const CHANNELS: { key: CurveChannel; label: string }[] = [
  { key: "master", label: "All channels" },
  { key: "red", label: "Red" },
  { key: "green", label: "Green" },
  { key: "blue", label: "Blue" },
];

const SIZE = 100;
const clamp = (v: number, low = 0, high = 1) => Math.min(high, Math.max(low, v));

/** How long after the last arrow key a run of nudges becomes one step, as with a slider. */
const NUDGE_PAUSE = 600;

/**
 * The tone curve. Click the line to add a point, drag points to bend it,
 * double-click a point to remove it. The two end points only move up and down.
 * From the keyboard each point takes the focus: the arrows move it, + adds
 * one after it, Delete removes it.
 */
export function CurveEditor() {
  const curves = useStore((s) => s.editor.adjustments.curves);
  const histogram = useStore((s) => s.editor.histogram);
  const [channel, setChannel] = useState<CurveChannel>("master");
  const id = useId();
  const svg = useRef<SVGSVGElement>(null);
  const dragging = useRef<number | null>(null);
  const points = curves[channel];
  // The point to give the focus to once the points are redrawn, after one is added or removed.
  const focusNext = useRef<number | null>(null);
  const pause = useRef<ReturnType<typeof setTimeout>>(undefined);
  useLayoutEffect(() => {
    const index = focusNext.current;
    focusNext.current = null;
    if (index !== null) svg.current?.querySelectorAll<SVGCircleElement>(".curve-point")[index]?.focus();
  }, [points]);

  const setPoints = (next: CurvePoints) => useStore.getState().adjust({ curves: { ...curves, [channel]: next } });

  const line = useMemo(() => {
    const at = curveSampler(points);
    let path = "";
    for (let i = 0; i <= 64; i++) {
      const x = i / 64;
      path += `${i === 0 ? "M" : "L"}${(x * SIZE).toFixed(2)} ${((1 - at(x)) * SIZE).toFixed(2)}`;
    }
    return path;
  }, [points]);

  // The brightness histogram as a backdrop, so you can see what a bend will affect.
  const backdrop = useMemo(() => {
    if (!histogram) return "";
    const bins = histogram.subarray(768, 1024);
    let peak = 1;
    for (let i = 2; i < 254; i++) peak = Math.max(peak, bins[i]);
    let path = `M0 ${SIZE}`;
    for (let i = 0; i < 256; i += 2) {
      const height = Math.min(1, Math.sqrt((bins[i] + bins[i + 1]) / (2 * peak)));
      path += `L${((i / 255) * SIZE).toFixed(1)} ${((1 - height * 0.9) * SIZE).toFixed(1)}`;
    }
    return `${path}L${SIZE} ${SIZE}Z`;
  }, [histogram]);

  const locate = (event: React.PointerEvent | React.MouseEvent): [number, number] => {
    const rect = svg.current!.getBoundingClientRect();
    return [clamp((event.clientX - rect.left) / rect.width), clamp(1 - (event.clientY - rect.top) / rect.height)];
  };

  const onPointerDown = (event: React.PointerEvent<SVGSVGElement>) => {
    if (event.button !== 0) return;
    const [x, y] = locate(event);
    const rect = svg.current!.getBoundingClientRect();
    // Grab a point within reach of the pointer, else add one where the click landed.
    let index = points.findIndex((p) => Math.hypot((p[0] - x) * rect.width, (p[1] - y) * rect.height) < 12);
    if (index < 0) {
      const tooClose = points.some((p) => Math.abs(p[0] - x) < 0.03);
      if (tooClose) return;
      const next: CurvePoints = [...points, [x, y] as [number, number]].sort((a, b) => a[0] - b[0]);
      index = next.findIndex((p) => p[0] === x);
      setPoints(next);
    }
    dragging.current = index;
    event.currentTarget.setPointerCapture(event.pointerId);
  };

  const onPointerMove = (event: React.PointerEvent) => {
    if (dragging.current !== null) setPoints(movePoint(points, dragging.current, locate(event)));
  };

  const endDrag = () => {
    if (dragging.current === null) return;
    dragging.current = null;
    useStore.getState().commitAdjust();
  };

  const remove = (index: number) => {
    setPoints(removePoint(points, index));
    useStore.getState().commitAdjust();
  };

  const commitNudges = () => {
    clearTimeout(pause.current);
    pause.current = undefined;
    useStore.getState().commitAdjust();
  };
  const onPointKey = (index: number) => (event: React.KeyboardEvent<SVGCircleElement>) => {
    if (event.ctrlKey || event.metaKey || event.altKey) return;
    if (event.key === "Escape") {
      // Off the point, as off a slider, rather than out of the photo.
      event.stopPropagation();
      event.currentTarget.blur();
      return;
    }
    if (event.key === "Delete" || event.key === "Backspace") {
      // Removes the point, and stops there: the photo isn't deleted.
      event.preventDefault();
      event.stopPropagation();
      commitNudges();
      focusNext.current = index === 0 || index === points.length - 1 ? index : index - 1;
      remove(index);
      return;
    }
    if (event.key === "+" || event.key === "=" || event.key === "Insert") {
      event.preventDefault();
      const added = addPointNear(points, index);
      if (!added) return;
      commitNudges();
      focusNext.current = added.index;
      setPoints(added.points);
      useStore.getState().commitAdjust();
      return;
    }
    const next = nudgePoint(points, index, event.key, event.shiftKey);
    if (!next) return;
    // The arrows would otherwise also change photo.
    event.preventDefault();
    event.stopPropagation();
    setPoints(next);
    clearTimeout(pause.current);
    pause.current = setTimeout(commitNudges, NUDGE_PAUSE);
  };

  return (
    <div className="curve-editor">
      <div
        className="channel-picker"
        role="tablist"
        aria-label="Curve channel"
        onKeyDown={(event) => onTabKey(event, (index) => setChannel(CHANNELS[index].key))}
      >
        {CHANNELS.map((c) => (
          <button
            key={c.key}
            id={`${id}-${c.key}`}
            role="tab"
            aria-selected={channel === c.key}
            aria-controls={`${id}-curve`}
            tabIndex={channel === c.key ? 0 : -1}
            aria-label={c.label}
            title={c.label}
            className={`channel ${c.key} ${channel === c.key ? "active" : ""} ${isStraight(curves[c.key]) ? "" : "bent"}`}
            onClick={() => setChannel(c.key)}
          />
        ))}
      </div>
      <svg
        ref={svg}
        id={`${id}-curve`}
        role="tabpanel"
        aria-labelledby={`${id}-${channel}`}
        className={`curve ${channel}`}
        viewBox={`0 0 ${SIZE} ${SIZE}`}
        preserveAspectRatio="none"
        onPointerDown={onPointerDown}
        onPointerMove={onPointerMove}
        onPointerUp={endDrag}
        onPointerCancel={endDrag}
      >
        <path d={backdrop} className="curve-backdrop" />
        <path d="M25 0V100M50 0V100M75 0V100M0 25H100M0 50H100M0 75H100" className="curve-grid" />
        <path d="M0 100L100 0" className="curve-grid" />
        <path d={line} className="curve-line" />
        {points.map(([x, y], index) => (
          <circle
            key={index}
            cx={x * SIZE}
            cy={(1 - y) * SIZE}
            r={2.6}
            className="curve-point"
            tabIndex={0}
            role="button"
            aria-label={`Point ${index + 1} of ${points.length}: arrow keys move it, + adds one, Delete removes it`}
            onDoubleClick={() => remove(index)}
            onKeyDown={onPointKey(index)}
            onBlur={() => pause.current !== undefined && commitNudges()}
          />
        ))}
      </svg>
    </div>
  );
}
