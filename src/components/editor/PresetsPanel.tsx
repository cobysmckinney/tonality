import { useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { Ellipsis, Plus, Star } from "lucide-react";
import { api } from "../../api";
import { changedSections, isOn, Preset, PRESET_SECTIONS } from "../../presets";
import { adjustablePreset, MenuEntry, presetStepLabel, useStore } from "../../store";
import { menuBelow } from "../Toolbar";
import { NameInput } from "./NameInput";
import { Slider } from "./Slider";

/** Saving the photo's edits as a preset: a name, and which groups of settings it covers. */
function NewPreset({ onDone }: { onDone: () => void }) {
  const [name, setName] = useState("");
  // Starts with the groups this photo's edits are in.
  const [sections, setSections] = useState(() => changedSections(useStore.getState().editor.adjustments));
  const complete = name.trim() !== "" && sections.length > 0;
  const toggle = (title: string) =>
    setSections(sections.includes(title) ? sections.filter((s) => s !== title) : [...sections, title]);
  // It opens at the foot of the list, which may be out of sight.
  const form = useRef<HTMLFormElement>(null);
  useEffect(() => form.current?.scrollIntoView({ block: "nearest" }), []);

  return (
    <form
      ref={form}
      className="preset-form"
      onSubmit={(event) => {
        event.preventDefault();
        if (!complete) return;
        void useStore.getState().savePreset(name, sections);
        onDone();
      }}
      onKeyDown={(event) => {
        if (event.key === "Escape") onDone();
        // Nothing typed or pressed here is one of the editor's shortcuts.
        event.stopPropagation();
      }}
    >
      <input
        className="branch-name"
        value={name}
        placeholder="Name this preset"
        aria-label="Preset name"
        autoFocus
        onChange={(event) => setName(event.currentTarget.value)}
      />
      <p className="panel-hint">Applying it changes only these groups of settings:</p>
      <div className="chips" role="group" aria-label="Groups the preset covers">
        {PRESET_SECTIONS.map(({ title }) => (
          <button
            key={title}
            type="button"
            aria-pressed={sections.includes(title)}
            className={`chip ${sections.includes(title) ? "active" : ""}`}
            onClick={() => toggle(title)}
          >
            {title}
          </button>
        ))}
      </div>
      <div className="preset-form-actions">
        <button type="button" className="button quiet" onClick={onDone}>
          Cancel
        </button>
        <button type="submit" className="button primary" disabled={!complete}>
          Save preset
        </button>
      </div>
    </form>
  );
}

/** A preview this long along the photo's longer side fills a tile sharply on a scaled screen. */
const PREVIEW_EDGE = 200;

/**
 * The open photo with each preset laid over its edits, by preset id. Drawn
 * again a moment after the edits stop changing.
 */
function usePreviews(): Map<number, ImageData> {
  const photoId = useStore((s) => s.editor.photoId);
  const ready = useStore((s) => s.editor.ready);
  const adjustments = useStore((s) => s.editor.adjustments);
  const presets = useStore((s) => s.presets);
  const [previews, setPreviews] = useState(new Map<number, ImageData>());
  useEffect(() => {
    if (!ready || photoId === null) return;
    let current = true;
    const timer = setTimeout(async () => {
      try {
        const drawn = await api.presetPreviews(photoId, adjustments, PREVIEW_EDGE);
        if (current) setPreviews(drawn);
      } catch {
        // The photo was closed meanwhile.
      }
    }, 400);
    return () => {
      current = false;
      clearTimeout(timer);
    };
  }, [photoId, ready, adjustments, presets]);
  // A different photo's previews would mislead.
  useEffect(() => setPreviews(new Map()), [photoId]);
  return previews;
}

function Preview({ pixels }: { pixels: ImageData | undefined }) {
  const canvas = useRef<HTMLCanvasElement>(null);
  useLayoutEffect(() => {
    const target = canvas.current;
    if (!target || !pixels) return;
    target.width = pixels.width;
    target.height = pixels.height;
    target.getContext("2d")!.putImageData(pixels, 0, 0);
  }, [pixels]);
  return <span className="preset-preview">{pixels && <canvas ref={canvas} />}</span>;
}

function PresetTile(props: {
  preset: Preset;
  preview: ImageData | undefined;
  on: boolean;
  favorite: boolean;
  renaming: boolean;
  onRename: (renaming: boolean) => void;
  /** The one tile Tab stops at; the arrow keys move between the rest. */
  reachable: boolean;
  onFocus: () => void;
}) {
  const { preset, on, favorite } = props;
  const s = useStore.getState();
  if (props.renaming) {
    return (
      <li className="preset-tile renaming">
        <NameInput
          name={preset.name}
          label="Preset name"
          onDone={(name) => {
            props.onRename(false);
            void s.renamePreset(preset.id, name);
          }}
        />
      </li>
    );
  }

  const own = preset.group === null;
  const star: MenuEntry = {
    label: favorite ? "Remove from favorites" : "Add to favorites",
    run: () => void s.toggleFavoritePreset(preset.id),
  };
  const entries: MenuEntry[] = own
    ? [
        star,
        "separator",
        { label: "Rename", run: () => props.onRename(true) },
        { label: "Update with this photo’s edits", run: () => void s.updatePreset(preset) },
        { label: "Export as a file…", run: () => void s.exportPreset(preset) },
        "separator",
        { label: "Delete", danger: true, run: () => void s.deletePreset(preset) },
      ]
    : [star];

  const tabIndex = props.reachable ? 0 : -1;
  return (
    <li className={`preset-tile ${on ? "on" : ""}`} onFocus={props.onFocus}>
      <button
        className="preset-main"
        aria-pressed={on}
        tabIndex={tabIndex}
        title={preset.name}
        // Pointing at a preset, or reaching it with the keyboard, tries it on
        // the photo. Once applied the photo shows what was made, not the trial.
        onMouseEnter={() => s.previewPreset(preset)}
        onMouseLeave={() => s.previewPreset(null)}
        onFocus={(event) => event.currentTarget.matches(":focus-visible") && s.previewPreset(preset)}
        onBlur={() => s.previewPreset(null)}
        onClick={() => void s.applyPreset([useStore.getState().openId!], preset)}
        onContextMenu={(event) => {
          event.preventDefault();
          s.openMenu(event.clientX, event.clientY, entries);
        }}
      >
        <Preview pixels={props.preview} />
        <span className="preset-name">{preset.name}</span>
      </button>
      <button
        className={`preset-star ${favorite ? "on" : ""}`}
        title={favorite ? "Remove from favorites" : "Add to favorites"}
        aria-label={favorite ? `Remove ${preset.name} from favorites` : `Add ${preset.name} to favorites`}
        aria-pressed={favorite}
        tabIndex={tabIndex}
        onClick={() => void s.toggleFavoritePreset(preset.id)}
      >
        <Star size={13} fill={favorite ? "currentColor" : "none"} />
      </button>
      {own && (
        <button
          className="preset-more"
          title="Rename, update, export or delete"
          aria-label={`More for ${preset.name}`}
          tabIndex={tabIndex}
          onClick={(event) => menuBelow(event, entries)}
        >
          <Ellipsis size={13} />
        </button>
      )}
    </li>
  );
}

/** The tile next to `from` in the grid, by where the tiles are on screen. */
function neighbourTile(tiles: HTMLElement[], from: HTMLElement, key: string): HTMLElement | undefined {
  const at = tiles.indexOf(from);
  if (key === "ArrowRight") return tiles[at + 1];
  if (key === "ArrowLeft") return tiles[at - 1];
  const here = from.getBoundingClientRect();
  const down = key === "ArrowDown";
  // The nearest row above or below, then the tile in it closest across.
  const candidates = tiles.filter((tile) => {
    const box = tile.getBoundingClientRect();
    return down ? box.top >= here.bottom - 1 : box.bottom <= here.top + 1;
  });
  const rowTop = down
    ? Math.min(...candidates.map((tile) => tile.getBoundingClientRect().top))
    : Math.max(...candidates.map((tile) => tile.getBoundingClientRect().top));
  return candidates
    .filter((tile) => Math.abs(tile.getBoundingClientRect().top - rowTop) < 2)
    .sort((a, b) => Math.abs(a.getBoundingClientRect().left - here.left) - Math.abs(b.getBoundingClientRect().left - here.left))[0];
}

/**
 * Presets: looks to lay over the photo's edits. Each covers some settings
 * and leaves the others alone. Each is shown on this photo; pointing at one
 * shows it large, and clicking applies it, as a step in the history, after
 * which its amount can be turned down or up. Starred ones also sit at the top.
 */
export function PresetsPanel() {
  const presets = useStore((s) => s.presets);
  const favorites = useStore((s) => s.favoritePresets);
  // Only once the photo being shown is the one loaded: a preset clicked
  // while the next photo loads must not land on the last one.
  const ready = useStore((s) => s.editor.ready && s.editor.photoId === s.openId);
  const adjustments = useStore((s) => s.editor.adjustments);
  const applied = useStore(adjustablePreset);
  const previews = usePreviews();
  const [adding, setAdding] = useState(false);
  const [renaming, setRenaming] = useState<string | null>(null);
  // Tab stops at one tile, with its star and "…", rather than at every preset.
  const [reached, setReached] = useState<string | null>(null);
  const list = useRef<HTMLDivElement>(null);
  const s = useStore.getState();

  // A trial never outlives the panel.
  useEffect(() => () => useStore.getState().previewPreset(null), []);

  const groups = useMemo(() => {
    const titles = [...new Set(presets.map((preset) => preset.group).filter((group) => group !== null))];
    const byId = new Map(presets.map((preset) => [preset.id, preset]));
    return [
      { title: "Favorites", presets: favorites.map((id) => byId.get(id)).filter((preset) => preset !== undefined) },
      ...titles.map((title) => ({ title, presets: presets.filter((preset) => preset.group === title) })),
    ].filter((group) => group.presets.length > 0);
  }, [presets, favorites]);
  const own = presets.filter((preset) => preset.group === null);
  const keys = [
    ...groups.flatMap((group) => group.presets.map((preset) => `${group.title}:${preset.id}`)),
    ...own.map((preset) => `Yours:${preset.id}`),
  ];
  const reachable = reached !== null && keys.includes(reached) ? reached : keys[0];

  // Renaming is keyed by group too: a favorite appears twice, and only the one clicked becomes a field.
  const tile = (group: string) => (preset: Preset) => {
    const key = `${group}:${preset.id}`;
    return (
      <PresetTile
        key={preset.id}
        preset={preset}
        preview={previews.get(preset.id)}
        on={isOn(preset, adjustments) || (applied?.preset.id === preset.id && applied.amount > 0)}
        favorite={favorites.includes(preset.id)}
        renaming={renaming === key}
        onRename={(yes) => setRenaming(yes ? key : null)}
        reachable={reachable === key}
        onFocus={() => setReached(key)}
      />
    );
  };

  // The arrow keys walk the grid, trying each preset on the photo as they go.
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (!["ArrowDown", "ArrowUp", "ArrowLeft", "ArrowRight"].includes(event.key)) return;
    const tiles = [...list.current!.querySelectorAll<HTMLButtonElement>(".preset-main")];
    // From a tile's star or "…" too, as from the tile.
    const from = (document.activeElement as HTMLElement).closest(".preset-tile")?.querySelector<HTMLButtonElement>(".preset-main");
    if (!from || !tiles.includes(from)) return;
    event.preventDefault();
    // Left and right would otherwise change photo.
    event.stopPropagation();
    const next = neighbourTile(tiles, from, event.key);
    next?.focus();
    next?.scrollIntoView({ block: "nearest" });
  };

  const addMenu = (event: React.MouseEvent) =>
    menuBelow(event, [
      { label: "Save this photo’s edits as a preset", run: () => setAdding(true) },
      { label: "Import presets from files…", run: () => void s.importPresets() },
    ]);

  return (
    <div className={`presets ${ready ? "" : "waiting"}`} inert={!ready}>
      <div className="preset-list" ref={list} onKeyDown={onKeyDown}>
        {groups.map((group) => (
          <section key={group.title}>
            <h3 className="preset-heading">{group.title}</h3>
            <ul className="preset-grid">{group.presets.map(tile(group.title))}</ul>
          </section>
        ))}
        <section>
          <h3 className="preset-heading">
            Yours
            <button className="icon-button small" title="Save a preset, or import some" aria-label="Add a preset" onClick={addMenu}>
              <Plus size={15} />
            </button>
          </h3>
          {adding && <NewPreset onDone={() => setAdding(false)} />}
          <ul className="preset-grid">{own.map(tile("Yours"))}</ul>
          {own.length === 0 && !adding && <p className="preset-empty">Save a photo’s edits here to use them again.</p>}
        </section>
      </div>

      {applied && (
        <footer className="preset-amount">
          <Slider
            label={`${applied.preset.name} amount`}
            value={applied.amount}
            min={0}
            max={200}
            origin={100}
            format={(value) => `${value}%`}
            onChange={s.setPresetAmount}
            commitLabel={() => {
              const standing = adjustablePreset(useStore.getState());
              return standing ? presetStepLabel(standing.preset, standing.amount) : undefined;
            }}
          />
        </footer>
      )}
    </div>
  );
}
