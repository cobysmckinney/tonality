import { ReactNode, useEffect, useId, useState } from "react";
import { ChevronDown, Pipette } from "lucide-react";
import { Band, DEFAULTS, FILM_KINDS, LABELS, MIXER_BANDS, rangeOf, Rgb, same, SECTIONS, SliderKey } from "../../adjustments";
import { api } from "../../api";
import { cssColour } from "../../film";
import { useStore } from "../../store";
import { onTabKey } from "../../tabs";
import { menuBelow } from "../Toolbar";
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
  const id = useId();
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
      <div className="band-picker" role="tablist" aria-label="Colour range" onKeyDown={(event) => onTabKey(event, setSelected)}>
        {MIXER_BANDS.map((b, i) => {
          const touched = mixer[i].hue !== 0 || mixer[i].saturation !== 0 || mixer[i].luminance !== 0;
          return (
            <button
              key={b.name}
              id={`${id}-${i}`}
              role="tab"
              aria-selected={selected === i}
              aria-controls={`${id}-panel`}
              tabIndex={selected === i ? 0 : -1}
              aria-label={b.name}
              title={b.name}
              className={`band ${selected === i ? "active" : ""} ${touched ? "bent" : ""}`}
              style={{ color: b.color }}
              onClick={() => setSelected(i)}
            />
          );
        })}
      </div>
      <div className="band-sliders" id={`${id}-panel`} role="tabpanel" aria-labelledby={`${id}-${selected}`}>
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
      </div>
    </>
  );
}

/** The film setting: turns a scan of a negative into a positive, then the rest works on that. */
function FilmTools() {
  const film = useStore((s) => s.editor.adjustments.film);
  const photoId = useStore((s) => s.editor.photoId);
  const picking = useStore((s) => s.editor.pickingBase);
  const s = useStore.getState();
  const kind = FILM_KINDS.find((k) => k.kind === film.kind)!;
  const [swatch, setSwatch] = useState<Rgb | null>(null);

  // The base in use, picked or guessed, for its swatch.
  const negative = film.kind !== "none";
  const base = film.base?.join();
  useEffect(() => {
    if (!negative || photoId === null) return setSwatch(null);
    let current = true;
    api
      .filmBase(photoId, useStore.getState().editor.adjustments)
      .then((answer) => current && setSwatch(answer.swatch))
      .catch(() => current && setSwatch(null));
    return () => {
      current = false;
    };
  }, [negative, base, photoId]);

  const kinds = (event: React.MouseEvent) =>
    menuBelow(
      event,
      FILM_KINDS.map((k) => ({
        label: k.name,
        checked: k.kind === film.kind,
        run: () => k.kind !== film.kind && s.setFilm({ kind: k.kind }),
      })),
    );

  return (
    <>
      <button className="button film-kind" aria-label={`Film: ${kind.name}`} onClick={kinds}>
        <span>{kind.name}</span>
        <ChevronDown size={14} />
      </button>
      {!negative && <p className="panel-hint">For scans of negatives. Slides and digital photos need nothing.</p>}
      {negative && (
        <div className="film-base">
          <span className="film-base-label">Film base</span>
          <span
            className="film-swatch"
            title={film.base ? "Picked from the photo" : "Guessed from the clear film"}
            style={swatch ? { background: cssColour(swatch) } : undefined}
          />
          {film.base && !film.range && (
            <button className="button quiet" title="Guess the film base from the scan again" onClick={() => s.setFilm({ base: null }, "Film base guessed")}>
              Auto
            </button>
          )}
          <button
            className={`button quiet ${picking ? "engaged" : ""}`}
            title="Click the clear film at the edge of a frame to set the base"
            aria-pressed={picking}
            onClick={() => s.setPickingBase(!picking)}
          >
            <Pipette size={14} /> Pick
          </button>
        </div>
      )}
      {negative && film.range && (
        <div className="film-roll">
          <p className="panel-hint">Balanced with the rest of its roll.</p>
          <button className="button quiet" onClick={() => s.setFilm({ range: null }, "Balanced on its own")}>
            Balance on its own
          </button>
        </div>
      )}
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
      <Section title="Film">
        <FilmTools />
      </Section>
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
