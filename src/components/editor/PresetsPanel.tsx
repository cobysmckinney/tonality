import { useEffect, useMemo, useRef, useState } from "react";
import { Ellipsis, Plus } from "lucide-react";
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

function PresetRow(props: { preset: Preset; on: boolean; renaming: boolean; onRename: (renaming: boolean) => void }) {
  const { preset, on } = props;
  const s = useStore.getState();
  if (props.renaming) {
    return (
      <li className="preset">
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
  const entries: MenuEntry[] = [
    { label: "Rename", run: () => props.onRename(true) },
    { label: "Update with this photo’s edits", run: () => void s.updatePreset(preset) },
    { label: "Export as a file…", run: () => void s.exportPreset(preset) },
    "separator",
    { label: "Delete", danger: true, run: () => void s.deletePreset(preset) },
  ];

  return (
    <li className={`preset ${on ? "on" : ""}`}>
      <button
        className="preset-main"
        aria-pressed={on}
        // Pointing at a preset, or reaching it with the keyboard, tries it on
        // the photo. Once applied the photo shows what was made, not the trial.
        onMouseEnter={() => s.previewPreset(preset)}
        onMouseLeave={() => s.previewPreset(null)}
        onFocus={(event) => event.currentTarget.matches(":focus-visible") && s.previewPreset(preset)}
        onBlur={() => s.previewPreset(null)}
        onClick={() => void s.applyPreset([useStore.getState().openId!], preset)}
        onContextMenu={(event) => own && s.openMenu(event.clientX, event.clientY, entries)}
      >
        <span className="preset-dot" />
        <span className="preset-name">{preset.name}</span>
      </button>
      {own && (
        <button className="icon-button small preset-more" title="Rename, update, export or delete" aria-label={`More for ${preset.name}`} onClick={(event) => menuBelow(event, entries)}>
          <Ellipsis size={14} />
        </button>
      )}
    </li>
  );
}

/**
 * Presets: looks to lay over the photo's edits. Each covers some settings
 * and leaves the others alone. The built-in ones come first, then your own.
 * Pointing at one shows it on the photo; clicking applies it, as a step in
 * the history, and its amount can then be turned down or up.
 */
export function PresetsPanel() {
  const presets = useStore((s) => s.presets);
  // Only once the photo being shown is the one loaded: a preset clicked
  // while the next photo loads must not land on the last one.
  const ready = useStore((s) => s.editor.ready && s.editor.photoId === s.openId);
  const adjustments = useStore((s) => s.editor.adjustments);
  const applied = useStore(adjustablePreset);
  const [adding, setAdding] = useState(false);
  const [renaming, setRenaming] = useState<number | null>(null);
  const list = useRef<HTMLDivElement>(null);
  const s = useStore.getState();

  // A trial never outlives the panel.
  useEffect(() => () => useStore.getState().previewPreset(null), []);

  const groups = useMemo(() => {
    const titles = [...new Set(presets.map((preset) => preset.group).filter((group) => group !== null))];
    return titles.map((title) => ({ title, presets: presets.filter((preset) => preset.group === title) }));
  }, [presets]);
  const own = presets.filter((preset) => preset.group === null);

  const row = (preset: Preset) => (
    <PresetRow
      key={preset.id}
      preset={preset}
      on={isOn(preset, adjustments) || (applied?.preset.id === preset.id && applied.amount > 0)}
      renaming={renaming === preset.id}
      onRename={(yes) => setRenaming(yes ? preset.id : null)}
    />
  );

  // Up and down walk the presets, trying each on the photo as they go.
  const onKeyDown = (event: React.KeyboardEvent) => {
    if (event.key !== "ArrowDown" && event.key !== "ArrowUp") return;
    const rows = [...list.current!.querySelectorAll<HTMLButtonElement>(".preset-main")];
    const at = rows.indexOf(document.activeElement as HTMLButtonElement);
    if (at < 0) return;
    event.preventDefault();
    rows[at + (event.key === "ArrowDown" ? 1 : -1)]?.focus();
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
            <ul>{group.presets.map(row)}</ul>
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
          <ul>{own.map(row)}</ul>
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
