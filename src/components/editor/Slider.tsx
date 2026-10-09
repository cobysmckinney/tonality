import { CSSProperties, useRef } from "react";
import { formatValue } from "../../adjustments";
import { useStore } from "../../store";

/** About as long as a double-click can take. */
const DOUBLE_CLICK = 400;

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
  /** How the value reads; a signed number unless given. */
  format?: (value: number) => string;
  onChange: (value: number) => void;
  /** What the history calls the step a change makes, when not the usual name for it; read as the change is recorded. */
  commitLabel?: () => string | undefined;
  /** What ends a change, when not recording it as a step in the history (the crop tool records its own). */
  onCommit?: () => void;
}

/**
 * One adjustment. Dragging changes the photo live; letting go makes the
 * whole drag a single undo step.
 */
export function Slider({ label, value, min, max, step = 1, origin = 0, track, format, onChange, commitLabel, onCommit }: Props) {
  const commit = () => (onCommit ? onCommit() : useStore.getState().commitAdjust(commitLabel?.()));
  // Arrow-key nudges arrive one at a time; a short pause ends the run, so a
  // burst of them undoes together rather than press by press.
  const pause = useRef<ReturnType<typeof setTimeout>>(undefined);
  const commitAfterPause = () => {
    clearTimeout(pause.current);
    pause.current = setTimeout(commit, 600);
  };
  // A click on the track jumps the slider there. Its step waits a moment, in
  // case it is the first half of a double-click to reset: then the reset
  // makes one step from where the slider started, not two.
  const pressed = useRef<{ x: number; y: number } | null>(null);
  const click = useRef<ReturnType<typeof setTimeout>>(undefined);
  const endPress = (event: React.PointerEvent) => {
    const from = pressed.current;
    pressed.current = null;
    clearTimeout(click.current);
    if (from && Math.hypot(event.clientX - from.x, event.clientY - from.y) < 4) click.current = setTimeout(commit, DOUBLE_CLICK);
    else commit();
  };
  const reset = () => {
    clearTimeout(click.current);
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
      <output className="slider-value">{format ? format(value) : formatValue(value, step)}</output>
      <input
        type="range"
        className={track ? "tinted" : ""}
        min={min}
        max={max}
        step={step}
        value={value}
        style={fill}
        onChange={(event) => onChange(Number(event.currentTarget.value))}
        onPointerDown={(event) => (pressed.current = { x: event.clientX, y: event.clientY })}
        onPointerUp={endPress}
        onKeyUp={commitAfterPause}
        onBlur={() => {
          clearTimeout(click.current);
          commit();
        }}
        onDoubleClick={reset}
      />
    </label>
  );
}
