import { useMemo, useRef, useState } from "react";
import { CurveChannel, CurvePoints, curveSampler, isStraight } from "../../adjustments";
import { useStore } from "../../store";

const CHANNELS: { key: CurveChannel; label: string }[] = [
  { key: "master", label: "All channels" },
  { key: "red", label: "Red" },
  { key: "green", label: "Green" },
  { key: "blue", label: "Blue" },
];

const SIZE = 100;
const clamp = (v: number, low = 0, high = 1) => Math.min(high, Math.max(low, v));

/**
 * The tone curve. Click the line to add a point, drag points to bend it,
 * double-click a point to remove it. The two end points only move up and down.
 */
export function CurveEditor() {
  const curves = useStore((s) => s.editor.adjustments.curves);
  const histogram = useStore((s) => s.editor.histogram);
  const [channel, setChannel] = useState<CurveChannel>("master");
  const svg = useRef<SVGSVGElement>(null);
  const dragging = useRef<number | null>(null);
  const points = curves[channel];

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

  const movePoint = (index: number, [x, y]: [number, number]) => {
    const last = points.length - 1;
    // Each point stays between its neighbours; the ends keep their place across.
    const fixedX = index === 0 ? 0 : index === last ? 1 : clamp(x, points[index - 1][0] + 0.02, points[index + 1][0] - 0.02);
    setPoints(points.map((p, i) => (i === index ? [fixedX, y] : p)));
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
    if (dragging.current !== null) movePoint(dragging.current, locate(event));
  };

  const endDrag = () => {
    if (dragging.current === null) return;
    dragging.current = null;
    useStore.getState().commitAdjust();
  };

  const removePoint = (index: number) => {
    const last = points.length - 1;
    if (index === 0 || index === last) {
      // End points can't be removed; put them back in their corner instead.
      setPoints(points.map((p, i) => (i === index ? [p[0], index === 0 ? 0 : 1] : p)));
    } else {
      setPoints(points.filter((_, i) => i !== index));
    }
    useStore.getState().commitAdjust();
  };

  return (
    <div className="curve-editor">
      <div className="channel-picker" role="tablist" aria-label="Curve channel">
        {CHANNELS.map((c) => (
          <button
            key={c.key}
            role="tab"
            aria-selected={channel === c.key}
            aria-label={c.label}
            title={c.label}
            className={`channel ${c.key} ${channel === c.key ? "active" : ""} ${isStraight(curves[c.key]) ? "" : "bent"}`}
            onClick={() => setChannel(c.key)}
          />
        ))}
      </div>
      <svg
        ref={svg}
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
            onDoubleClick={() => removePoint(index)}
          />
        ))}
      </svg>
    </div>
  );
}
