import { CSSProperties, useRef } from "react";
import { formatValue } from "../../adjustments";
import { useStore } from "../../store";

interface Props {
  label: string;
  value: number;
  min: number;
  max: number;
  step?: number;
  /** Where the slider rests; double-clicking returns to it. */
  origin?: number;
  /** A CSS gradient for sliders whose track shows what they do (temperature, hue). */
  track?: string;
  onChange: (value: number) => void;
}

/**
 * One adjustment. Dragging changes the photo live; letting go makes the
 * whole drag a single undo step.
 */
export function Slider({ label, value, min, max, step = 1, origin = 0, track, onChange }: Props) {
  const commit = () => useStore.getState().commitAdjust();
  // Arrow-key nudges arrive one at a time; a short pause ends the run, so a
  // burst of them undoes together rather than press by press.
  const pause = useRef<ReturnType<typeof setTimeout>>(undefined);
  const commitAfterPause = () => {
    clearTimeout(pause.current);
    pause.current = setTimeout(commit, 600);
  };
  const reset = () => {
    onChange(origin);
    commit();
  };
  const percent = (v: number) => `${((v - min) / (max - min)) * 100}%`;
  const fill = {
    "--from": percent(Math.min(value, origin)),
    "--to": percent(Math.max(value, origin)),
    ...(track ? { "--track": track } : {}),
  } as CSSProperties;

  return (
    <label className={`slider ${value !== origin ? "changed" : ""}`}>
      <span className="slider-label" onDoubleClick={reset}>
        {label}
      </span>
      <output className="slider-value">{formatValue(value, step)}</output>
      <input
        type="range"
        className={track ? "tinted" : ""}
        min={min}
        max={max}
        step={step}
        value={value}
        style={fill}
        onChange={(event) => onChange(Number(event.currentTarget.value))}
        onPointerUp={commit}
        onKeyUp={commitAfterPause}
        onBlur={commit}
        onDoubleClick={reset}
      />
    </label>
  );
}
