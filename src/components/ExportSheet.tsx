import { CSSProperties, useEffect, useRef, useState } from "react";
import { Check, Folder, FolderOpen, RotateCcw } from "lucide-react";
import { ExportJob, ExportPlan, ExportSettings, ExportSummary } from "../api";
import { count, fileSize, plural } from "../format";
import { ExportState, useStore } from "../store";

const FORMATS: { value: ExportSettings["format"]; label: string; extension: string; about: string }[] = [
  { value: "jpeg", label: "JPEG", extension: "jpg", about: "Small files that open anywhere. For sharing and the web." },
  { value: "png", label: "PNG", extension: "png", about: "Nothing lost to compression, at several times the size." },
  { value: "tiff", label: "TIFF", extension: "tif", about: "16-bit and uncompressed, for more editing elsewhere. Large files." },
];

/** The longest side of the exported picture; `null` keeps every pixel. */
const SIZES: { value: number | null; label: string }[] = [
  { value: null, label: "Full size" },
  { value: 4096, label: "4096 px" },
  { value: 2560, label: "2560 px" },
  { value: 2048, label: "2048 px" },
];

/** What a name for several files can be built from. */
const PLACEHOLDERS = [
  { token: "{name}", label: "Photo name" },
  { token: "{branch}", label: "Branch" },
  { token: "{date}", label: "Date taken" },
  { token: "{n}", label: "Number" },
];

/** What the name field says when several files are left to their own names. */
const NO_NAME = "Named after each photo";

/** One choice out of a few, all on show. */
function Segments<T>(props: { label: string; options: { value: T; label: string }[]; value: T; onChoose: (value: T) => void }) {
  return (
    <div className="segments" role="radiogroup" aria-label={props.label}>
      {props.options.map((option) => (
        <button
          key={option.label}
          role="radio"
          aria-checked={option.value === props.value}
          className={option.value === props.value ? "active" : ""}
          onClick={() => props.onChoose(option.value)}
        >
          {option.label}
        </button>
      ))}
    </div>
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

const pixels = (file: { width: number | null; height: number | null }) =>
  file.width !== null && file.height !== null ? `${file.width} × ${file.height}` : null;

/** Where the files go: the folder's name, and the whole path it sits in so there is no guessing. */
function Destination({ folder, onChange }: { folder: string; onChange?: () => void }) {
  const cut = Math.max(folder.lastIndexOf("/"), folder.lastIndexOf("\\"));
  return (
    <div className="export-destination">
      <Folder size={15} />
      <div className="export-folder">
        <strong>{folder.slice(cut + 1)}</strong>
        {cut > 0 && <span>in {folder.slice(0, cut)}</span>}
      </div>
      {onChange && (
        <button className="button" onClick={onChange}>
          Change…
        </button>
      )}
    </div>
  );
}

/**
 * What the files will be called. One file has a name, typed like any other;
 * several share a name built from placeholders, and the list underneath
 * shows what each comes out as.
 */
function Naming({ job, plan }: { job: ExportJob; plan: ExportPlan }) {
  const input = useRef<HTMLInputElement>(null);
  const { changeExport } = useStore.getState();
  const { files } = plan;
  const extension = FORMATS.find((format) => format.value === plan.settings.format)!.extension;
  const only = files.length === 1 ? files[0] : null;
  // Left alone, the field of a single file shows the name it is going to get.
  const value = job.name ?? only?.name.replace(/\.[^.]+$/, "") ?? "";
  const numbered = files.filter((file) => file.numbered).length;

  const insert = (token: string) => {
    const field = input.current!;
    const [from, to] = [field.selectionStart ?? value.length, field.selectionEnd ?? value.length];
    void changeExport({ name: value.slice(0, from) + token + value.slice(to) });
    // Carry on typing right after what was put in.
    requestAnimationFrame(() => {
      field.focus();
      field.setSelectionRange(from + token.length, from + token.length);
    });
  };

  return (
    <>
      <div className="export-name">
        {/* The field is as wide as what is in it, so the extension follows the name. */}
        <span className="export-name-field" data-text={value || NO_NAME}>
          <input
            ref={input}
            // Otherwise an input insists on room for twenty characters.
            size={1}
            value={value}
            placeholder={NO_NAME}
            aria-label={only ? "File name" : "Name for the files"}
            spellCheck={false}
            onChange={(event) => void changeExport({ name: event.currentTarget.value })}
          />
        </span>
        <span className="export-extension">.{extension}</span>
        {job.name !== null && (
          <button
            className="icon-button small"
            title={only ? "Back to the photo’s own name" : "Back to each photo’s own name"}
            aria-label="Reset the name"
            onClick={() => void changeExport({ name: null })}
          >
            <RotateCcw size={13} />
          </button>
        )}
      </div>

      {only ? (
        <div className="export-facts">
          {pixels(only) && <span>{pixels(only)} px</span>}
          {only.name !== `${value}.${extension}` && (
            <span>
              {only.numbered ? "That name is taken in this folder, so the file will be " : "Will be saved as "}
              <strong>{only.name}</strong>
              {only.numbered && ". Nothing is replaced."}
            </span>
          )}
        </div>
      ) : (
        <>
          <div className="export-placeholders">
            <span>Add</span>
            {PLACEHOLDERS.map((placeholder) => (
              <button key={placeholder.token} className="chip" onClick={() => insert(placeholder.token)}>
                {placeholder.label}
              </button>
            ))}
          </div>
          <ul className="export-files" aria-label="Files this export will write">
            {files.map((file) => (
              <li key={file.name}>
                <span className="export-file-name">{file.name}</span>
                {file.branch !== null && <span>{file.branch}</span>}
                <span>{pixels(file)}</span>
              </li>
            ))}
          </ul>
          {numbered > 0 && (
            <div className="export-facts">
              <span>
                {plural(numbered, "name is", "names are")} already taken, here or by an earlier file in this list, so{" "}
                {numbered === 1 ? "that file is" : "those files are"} numbered. Nothing is replaced.
              </span>
            </div>
          )}
        </>
      )}
    </>
  );
}

/** Which branches to export: any of one photo's, or for several photos, the one each is on or all of them. */
function Edits({ job, plan }: { job: ExportJob; plan: ExportPlan }) {
  const { changeExport } = useStore.getState();

  if (job.ids.length > 1) {
    if (!plan.severalBranches) return null;
    return (
      <Row label="Edits">
        <Segments<"current" | "all">
          label="Branches to export"
          options={[
            { value: "current", label: "Current branch" },
            { value: "all", label: "All branches" },
          ]}
          value={job.branches === "all" ? "all" : "current"}
          onChoose={(branches) => void changeExport({ branches })}
        />
        <span className="export-note">
          {job.branches === "all"
            ? "A file for every branch of every photo, each named for its branch."
            : "Each photo as the grid shows it: the branch it is on."}
        </span>
      </Row>
    );
  }

  if (plan.branches.length < 2) return null;
  const chosen = new Set(plan.files.map((file) => file.branchId));
  const current = plan.branches.find((branch) => branch.id === plan.currentBranchId);
  const toggle = (id: number) => {
    const next = plan.branches.map((branch) => branch.id).filter((other) => (other === id) !== chosen.has(other));
    // At least one branch is always exported.
    if (next.length > 0) void changeExport({ branches: { chosen: next } });
  };
  return (
    <Row label="Edits">
      <div className="chips" role="group" aria-label="Branches to export">
        {plan.branches.map((branch) => (
          <button
            key={branch.id}
            className={`chip ${chosen.has(branch.id) ? "active" : ""}`}
            aria-pressed={chosen.has(branch.id)}
            onClick={() => toggle(branch.id)}
          >
            {chosen.has(branch.id) && <Check size={12} strokeWidth={3} />}
            {branch.name}
          </button>
        ))}
      </div>
      <span className="export-note">
        {chosen.size > 1
          ? `A file for each of these ${count(chosen.size)} branches.`
          : chosen.has(plan.currentBranchId)
            ? "The branch this photo is showing now."
            : `Not the branch this photo is showing now, which is “${current?.name}”.`}
      </span>
    </Row>
  );
}

function Size({ longEdge, files }: { longEdge: number | null; files: ExportPlan["files"] }) {
  const { changeExport } = useStore.getState();
  // "Custom" stays chosen even while its box holds a number that happens to be a preset.
  const [custom, setCustom] = useState(!SIZES.some((size) => size.value === longEdge));
  // What is in the box, which may be half-typed.
  const [typed, setTyped] = useState(String(longEdge ?? ""));
  const set = (value: number | null) => void changeExport({ settings: { longEdge: value } });
  const shrunk = files.filter((file) => file.shrunk).length;

  return (
    <Row label="Size">
      <Segments
        label="Size"
        options={[...SIZES, { value: -1, label: "Custom" }]}
        value={custom ? -1 : longEdge}
        onChoose={(value) => {
          setCustom(value === -1);
          if (value !== -1) return set(value);
          const start = longEdge ?? 3000;
          setTyped(String(start));
          set(start);
        }}
      />
      {custom && (
        <label className="export-pixels">
          <input
            type="number"
            min={1}
            value={typed}
            aria-label="Longest side in pixels"
            onChange={(event) => {
              setTyped(event.currentTarget.value);
              const pixels = Math.round(Number(event.currentTarget.value));
              if (pixels >= 1) set(pixels);
            }}
          />
          px
        </label>
      )}
      {longEdge !== null && <span className="export-note">Along the longer side. Smaller pictures are not enlarged.</span>}
      {shrunk > 0 && (
        <span className="export-note">
          {files.length === 1
            ? "This photo is"
            : shrunk === files.length
              ? "These photos are"
              : plural(shrunk, "of these photos is", "of these photos are")}{" "}
          larger than this computer’s graphics card can hold, so {shrunk === 1 ? "it comes" : "they come"} out smaller.
        </span>
      )}
    </Row>
  );
}

/** Before exporting: what is about to be written and where, with everything that shapes it underneath. */
function Setup({ job, plan }: Extract<ExportState, { phase: "setup" }>) {
  const photo = useStore((s) => (job.ids.length === 1 ? s.photos.find((p) => p.id === job.ids[0]) : undefined));
  const { changeExport, chooseExportFolder, runExport, dismissExport } = useStore.getState();
  const { settings, files } = plan;
  const format = FORMATS.find((option) => option.value === settings.format)!;
  const fill = { "--from": "0%", "--to": `${((settings.quality - 1) / 99) * 100}%` } as CSSProperties;

  return (
    <div className="panel dialog export" role="dialog" aria-labelledby="export-title">
      <h2 id="export-title">{photo ? `Export ${photo.fileName}` : `Export ${plural(job.ids.length, "photo")}`}</h2>

      <section className="export-output">
        <Naming job={job} plan={plan} />
        <Destination folder={plan.folder} onChange={() => void chooseExportFolder()} />
      </section>

      <dl className="export-rows">
        <Edits job={job} plan={plan} />
        <Row label="Format">
          <Segments label="Format" options={FORMATS} value={settings.format} onChoose={(value) => void changeExport({ settings: { format: value } })} />
          <span className="export-note">{format.about}</span>
        </Row>
        {settings.format === "jpeg" && (
          <Row label="Quality">
            <label className="slider export-quality">
              <input
                type="range"
                min={1}
                max={100}
                value={settings.quality}
                style={fill}
                aria-label="JPEG quality"
                onChange={(event) => void changeExport({ settings: { quality: Number(event.currentTarget.value) } })}
              />
              <output>{settings.quality}</output>
            </label>
          </Row>
        )}
        <Size longEdge={settings.longEdge} files={files} />
      </dl>

      <div className="dialog-actions">
        <button className="button" onClick={() => void dismissExport()}>
          Cancel
        </button>
        <button className="button primary" autoFocus onClick={() => void runExport()}>
          {files.length === 1 ? "Export" : `Export ${plural(files.length, "file")}`}
        </button>
      </div>
    </div>
  );
}

/** After exporting: what was written and where, until it is dismissed. */
function Done({ summary }: { summary: ExportSummary }) {
  const { exported, failed } = summary;
  const { dismissExport, reveal } = useStore.getState();
  const one = exported.length === 1 ? exported[0] : null;

  return (
    <div className="panel dialog export" role="alertdialog" aria-labelledby="export-title">
      <h2 id="export-title">
        {exported.length === 0 ? "Nothing was exported" : one ? "Exported" : `Exported ${plural(exported.length, "file")}`}
      </h2>
      {summary.cancelled && <p>The export was stopped before it finished.</p>}

      {exported.length > 0 && (
        <section className="export-output">
          {one ? (
            <>
              <div className="export-name written">{one.name}</div>
              <div className="export-facts">
                <span>
                  {one.width} × {one.height} px
                </span>
                <span>{fileSize(one.fileSize)}</span>
                {one.branch !== null && <span>The “{one.branch}” branch</span>}
              </div>
            </>
          ) : (
            <ul className="export-files">
              {exported.map((file) => (
                <li key={file.path}>
                  <span className="export-file-name">{file.name}</span>
                  {file.branch !== null && <span>{file.branch}</span>}
                  <span>{fileSize(file.fileSize)}</span>
                </li>
              ))}
            </ul>
          )}
          <Destination folder={summary.folder} />
        </section>
      )}

      {failed.length > 0 && (
        <>
          <p>{plural(failed.length, "file")} couldn’t be exported:</p>
          <ul className="failures">
            {failed.map((failure, index) => (
              <li key={index}>
                <strong>{failure.fileName}</strong>
                <span>{failure.reason}</span>
              </li>
            ))}
          </ul>
        </>
      )}

      <div className="dialog-actions">
        {exported.length > 0 && (
          <button className="button" onClick={() => void reveal(exported[0].path)}>
            <FolderOpen size={15} /> Show in file manager
          </button>
        )}
        <button className="button primary" autoFocus onClick={() => void dismissExport()}>
          Done
        </button>
      </div>
    </div>
  );
}

/** The export flow: what is about to be written, the writing, and what was written. */
export function ExportSheet() {
  const state = useStore((s) => s.exportState);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const current = useStore.getState();
      const phase = current.exportState?.phase;
      if (!phase) return;
      if (event.key === "Escape") void current.dismissExport();
      // Enter is the sheet's main action unless a button has the focus.
      if (event.key === "Enter" && !(event.target as HTMLElement).closest("button")) {
        if (phase === "setup") void current.runExport();
        else if (phase === "done") void current.dismissExport();
      }
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (!state) return null;
  const { dismissExport } = useStore.getState();
  return (
    <div
      className="scrim"
      onMouseDown={(event) => event.target === event.currentTarget && state.phase !== "exporting" && void dismissExport()}
    >
      {state.phase === "setup" && <Setup {...state} />}
      {state.phase === "exporting" && (
        <div className="panel dialog" role="status">
          <h2>{state.progress.total === 1 ? "Exporting…" : `Exporting ${plural(state.progress.total, "file")}…`}</h2>
          {state.progress.total === 1 ? (
            <>
              <p>Drawing the picture and writing the file.</p>
              <div className="export-progress">
                <div className="progress waiting" role="progressbar" aria-label="Exporting">
                  <div />
                </div>
              </div>
            </>
          ) : (
            <>
              <div className="export-progress">
                <div
                  className="progress"
                  role="progressbar"
                  aria-valuemin={0}
                  aria-valuemax={state.progress.total}
                  aria-valuenow={state.progress.done}
                >
                  <div style={{ width: `${(state.progress.done / state.progress.total) * 100}%` }} />
                </div>
                <span className="progress-label">
                  {count(state.progress.done)} of {count(state.progress.total)}
                </span>
              </div>
              <div className="dialog-actions">
                <button className="button" onClick={() => void dismissExport()}>
                  Stop
                </button>
              </div>
            </>
          )}
        </div>
      )}
      {state.phase === "done" && <Done summary={state.summary} />}
    </div>
  );
}
