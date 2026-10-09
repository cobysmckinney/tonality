import { useEffect, useId, useMemo, useRef, useState } from "react";
import {
  applyForm,
  changesAnything,
  digitsOnly,
  FilmForm,
  FilmPhoto,
  FilmSuggestions,
  hasFilm,
  NO_FILM,
  startingForm,
  suggestionsFor,
} from "../film";
import { plural } from "../format";
import { useStore } from "../store";

/** A text field that offers what has been typed in before, the most used first. */
function Suggest(props: {
  label: string;
  value: string;
  options: string[];
  placeholder: string;
  autoFocus?: boolean;
  onChange: (value: string) => void;
}) {
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const list = useId();
  const shown = open ? suggestionsFor(props.options, props.value) : [];
  const choose = (option: string) => {
    props.onChange(option);
    setOpen(false);
    setActive(-1);
  };

  return (
    <div className="suggest">
      <input
        className="film-input"
        value={props.value}
        placeholder={props.placeholder}
        aria-label={props.label}
        role="combobox"
        aria-expanded={shown.length > 0}
        aria-controls={list}
        aria-autocomplete="list"
        aria-activedescendant={active >= 0 ? `${list}-${active}` : undefined}
        autoFocus={props.autoFocus}
        spellCheck={false}
        // Offered on a click, while typing, or with the down arrow; not just
        // for arriving in the field, where the list would cover the next one.
        onMouseDown={() => setOpen(true)}
        onBlur={() => setOpen(false)}
        onChange={(event) => {
          props.onChange(event.currentTarget.value);
          setOpen(true);
          setActive(-1);
        }}
        onKeyDown={(event) => {
          if (!open && event.key === "ArrowDown") {
            event.preventDefault();
            setOpen(true);
            return;
          }
          if (shown.length === 0) return;
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            const last = shown.length - 1;
            if (event.key === "ArrowDown") setActive((at) => (at >= last ? 0 : at + 1));
            else setActive((at) => (at <= 0 ? last : at - 1));
          } else if (event.key === "Enter" && active >= 0) {
            // Takes the suggestion rather than saving the sheet.
            event.preventDefault();
            event.stopPropagation();
            choose(shown[active]);
          } else if (event.key === "Escape") {
            // Closes the suggestions, not the sheet.
            event.stopPropagation();
            setOpen(false);
          }
        }}
      />
      {shown.length > 0 && (
        <ul className="suggestions" id={list} role="listbox" aria-label={props.label}>
          {shown.map((option, i) => (
            <li
              key={option}
              id={`${list}-${i}`}
              role="option"
              aria-selected={i === active}
              className={i === active ? "active" : ""}
              // Before the field loses the focus, which would close the list.
              onMouseDown={(event) => {
                event.preventDefault();
                choose(option);
              }}
            >
              {option}
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

function NumberField(props: { label: string; value: string; placeholder: string; onChange: (value: string) => void }) {
  return (
    <input
      className="film-input film-number"
      inputMode="numeric"
      value={props.value}
      placeholder={props.placeholder}
      aria-label={props.label}
      onChange={(event) => props.onChange(digitsOnly(event.currentTarget.value))}
    />
  );
}

function Row({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="export-row">
      <dt>{label}</dt>
      <dd>{children}</dd>
    </div>
  );
}

const MIXED = "Mixed";

function Setup({ photos, suggestions }: { photos: FilmPhoto[]; suggestions: FilmSuggestions }) {
  const { saveFilmDetails, closeFilmDetails } = useStore.getState();
  const start = useMemo(() => startingForm(photos), [photos]);
  const [form, setForm] = useState<FilmForm>(start.form);
  const only = photos.length === 1 ? photos[0] : null;
  const field = (key: keyof FilmForm) => ({
    value: form[key],
    onChange: (value: string) => setForm((current) => ({ ...current, [key]: value })),
  });
  const placeholder = (key: keyof FilmForm, example: string) => (start.mixed.has(key) ? MIXED : example);

  // The sheet's keys: Enter saves unless a button has the focus, Esc closes.
  const save = useRef(() => {});
  save.current = () => {
    const after = applyForm(photos, start.form, form);
    if (changesAnything(photos, after)) void saveFilmDetails(after);
    else closeFilmDetails();
  };
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key === "Escape") closeFilmDetails();
      if (event.key === "Enter" && !(event.target as HTMLElement).closest("button")) save.current();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, [closeFilmDetails]);

  return (
    <div className="panel dialog film" role="dialog" aria-modal="true" aria-labelledby="film-title">
      <h2 id="film-title">{only ? `Film details for ${only.fileName}` : `Film details for ${plural(photos.length, "photo")}`}</h2>
      <p>What the film was and the camera it was shot with. Info and exports show these instead of the scanning camera’s.</p>

      <dl className="export-rows film-rows">
        <Row label="Film">
          <Suggest label="Film" options={suggestions.stocks} placeholder={placeholder("stock", "Kodak Portra 400")} autoFocus {...field("stock")} />
        </Row>
        <Row label="ISO">
          <NumberField label="ISO" placeholder={placeholder("iso", "400")} {...field("iso")} />
        </Row>
        <Row label="Camera">
          <Suggest label="Camera" options={suggestions.cameras} placeholder={placeholder("camera", "Nikon FM2")} {...field("camera")} />
        </Row>
        <Row label="Lens">
          <Suggest label="Lens" options={suggestions.lenses} placeholder={placeholder("lens", "Nikkor 50mm f/1.4")} {...field("lens")} />
        </Row>
        <Row label={only ? "Frame" : "First frame"}>
          <NumberField
            label={only ? "Frame" : "First frame"}
            placeholder={start.mixed.has("frame") ? "Each keeps its own" : only ? "" : "1"}
            {...field("frame")}
          />
          {!only && <span className="export-note">Frames count up from here, in the order they were scanned.</span>}
        </Row>
      </dl>

      {start.mixed.size > 0 && <p className="film-mixed">Fields marked Mixed keep each photo’s own unless you type in them.</p>}

      <div className="dialog-actions">
        <button
          className="button quiet film-clear"
          disabled={!photos.some((photo) => hasFilm(photo.film))}
          onClick={() => void saveFilmDetails(photos.map((photo) => ({ ...photo, film: NO_FILM })))}
        >
          Clear details
        </button>
        <button className="button" onClick={closeFilmDetails}>
          Cancel
        </button>
        <button className="button primary" onClick={() => save.current()}>
          Save
        </button>
      </div>
    </div>
  );
}

/** Film details for one photo or a roll of them: the stock, ISO, camera, lens and frame. */
export function FilmSheet() {
  const sheet = useStore((s) => s.filmSheet);
  if (!sheet) return null;
  return (
    <div className="scrim" onMouseDown={(event) => event.target === event.currentTarget && useStore.getState().closeFilmDetails()}>
      <Setup photos={sheet.photos} suggestions={sheet.suggestions} />
    </div>
  );
}
