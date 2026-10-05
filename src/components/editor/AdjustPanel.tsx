import { ReactNode, useState } from "react";
import { ChevronDown } from "lucide-react";
import { Band, DEFAULTS, LABELS, MIXER_BANDS, rangeOf, same, SECTIONS, SliderKey } from "../../adjustments";
import { useStore } from "../../store";
import { CurveEditor } from "./CurveEditor";
import { Slider } from "./Slider";

/** A collapsible group of tools, with a reset that appears once something in it has changed. */
function Section(props: { title: string; children: ReactNode }) {
  const [open, setOpen] = useState(true);
  const keys = SECTIONS.find((section) => section.title === props.title)!.keys;
  const changed = useStore((s) => keys.some((key) => !same(s.editor.adjustments[key], DEFAULTS[key])));
  const reset = () => {
    const { adjust, commitAdjust } = useStore.getState();
    adjust(Object.fromEntries(keys.map((key) => [key, DEFAULTS[key]])));
    commitAdjust();
  };

  return (
    <section className={`tool-section ${open ? "open" : ""}`}>
      <header>
        <button className="section-title" aria-expanded={open} onClick={() => setOpen(!open)}>
          <ChevronDown size={14} />
          {props.title}
        </button>
        {changed && (
          <button className="section-reset" onClick={reset}>
            Reset
          </button>
        )}
      </header>
      {open && <div className="section-body">{props.children}</div>}
    </section>
  );
}

/** A slider bound to one numeric adjustment. */
function Tool(props: { name: SliderKey; track?: string }) {
  const value = useStore((s) => s.editor.adjustments[props.name]);
  return (
    <Slider
      label={LABELS[props.name]}
      value={value}
      {...rangeOf(props.name)}
      track={props.track}
      onChange={(next) => useStore.getState().adjust({ [props.name]: next })}
    />
  );
}

const HUE_STEPS = [0, 30, 60, 120, 180, 240, 280, 320, 360];

/** The colour mixer: pick a colour range, then shift, saturate or brighten just that range. */
function Mixer() {
  const mixer = useStore((s) => s.editor.adjustments.mixer);
  const [selected, setSelected] = useState(0);
  const band = mixer[selected];
  const change = (part: Partial<Band>) =>
    useStore.getState().adjust({ mixer: mixer.map((b, i) => (i === selected ? { ...b, ...part } : b)) });

  // The hue track runs from the neighbouring colour on one side to the one on the other.
  const hue = HUE_STEPS[selected];
  const hueTrack = `linear-gradient(to right, hsl(${hue - 30} 70% 55%), hsl(${hue} 70% 55%), hsl(${hue + 30} 70% 55%))`;
  const saturationTrack = `linear-gradient(to right, hsl(${hue} 0% 50%), hsl(${hue} 80% 55%))`;
  const luminanceTrack = `linear-gradient(to right, hsl(${hue} 60% 18%), hsl(${hue} 70% 75%))`;

  return (
    <>
      <div className="band-picker" role="tablist" aria-label="Colour range">
        {MIXER_BANDS.map((b, i) => {
          const touched = mixer[i].hue !== 0 || mixer[i].saturation !== 0 || mixer[i].luminance !== 0;
          return (
            <button
              key={b.name}
              role="tab"
              aria-selected={selected === i}
              aria-label={b.name}
              title={b.name}
              className={`band ${selected === i ? "active" : ""} ${touched ? "bent" : ""}`}
              style={{ color: b.color }}
              onClick={() => setSelected(i)}
            />
          );
        })}
      </div>
      <Slider label="Hue" value={band.hue} min={-100} max={100} track={hueTrack} onChange={(v) => change({ hue: v })} />
      <Slider
        label="Saturation"
        value={band.saturation}
        min={-100}
        max={100}
        track={saturationTrack}
        onChange={(v) => change({ saturation: v })}
      />
      <Slider
        label="Luminance"
        value={band.luminance}
        min={-100}
        max={100}
        track={luminanceTrack}
        onChange={(v) => change({ luminance: v })}
      />
    </>
  );
}

const TEMPERATURE_TRACK = "linear-gradient(to right, #4f8fe6, #b9b9b9, #f0a63c)";
const TINT_TRACK = "linear-gradient(to right, #4fb866, #b9b9b9, #d756b8)";

/** Every editing tool, grouped the way you'd work through a photo. */
export function AdjustPanel() {
  const ready = useStore((s) => s.editor.ready);
  return (
    <div className={`adjust ${ready ? "" : "waiting"}`} inert={!ready}>
      <Section title="Light">
        <Tool name="exposure" />
        <Tool name="contrast" />
        <Tool name="highlights" />
        <Tool name="shadows" />
        <Tool name="whites" />
        <Tool name="blacks" />
      </Section>
      <Section title="Color">
        <Tool name="temperature" track={TEMPERATURE_TRACK} />
        <Tool name="tint" track={TINT_TRACK} />
        <Tool name="vibrance" />
        <Tool name="saturation" />
      </Section>
      <Section title="Curve">
        <CurveEditor />
      </Section>
      <Section title="Color mixer">
        <Mixer />
      </Section>
      <Section title="Detail">
        <Tool name="sharpening" />
        <Tool name="noiseReduction" />
        <Tool name="clarity" />
        <Tool name="dehaze" />
      </Section>
      <Section title="Effects">
        <Tool name="vignette" />
        <Tool name="grain" />
      </Section>
    </div>
  );
}
