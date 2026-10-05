import { memo, useEffect, useMemo, useState } from "react";
import { Check, ImageOff } from "lucide-react";
import { ScanItem, scanThumbUrl } from "../api";
import { count, plural } from "../format";
import { ImportState, useStore } from "../store";
import { GridSection, VirtualGrid } from "./VirtualGrid";

const itemKey = (item: ScanItem) => item.index;

const Candidate = memo(function Candidate(props: {
  item: ScanItem;
  sessionId: number;
  chosen: boolean;
  locked: boolean;
}) {
  const { item, sessionId, chosen, locked } = props;
  const [failed, setFailed] = useState(false);
  const duplicate = item.status === "duplicate";

  return (
    <button
      className={`photo candidate ${chosen ? "chosen" : ""} ${duplicate ? "duplicate" : ""}`}
      title={item.fileName}
      disabled={duplicate || locked}
      aria-pressed={chosen}
      onClick={() => useStore.getState().toggleChosen([item.index], !chosen)}
    >
      {failed ? (
        <div className="photo-missing">
          <ImageOff size={20} />
          <span>{item.fileName}</span>
        </div>
      ) : (
        <img src={scanThumbUrl(sessionId, item.index)} alt="" draggable={false} decoding="async" onError={() => setFailed(true)} />
      )}
      {!duplicate && <span className="tick">{chosen && <Check size={12} strokeWidth={3} />}</span>}
      {item.hasJpeg && <span className="badge pair">RAW + JPEG</span>}
    </button>
  );
});

function SectionHeading({ title, detail }: { title: string; detail?: string }) {
  return (
    <header className="day-heading compact">
      <h2>{title}</h2>
      {detail && <span className="day-detail">{detail}</span>}
    </header>
  );
}

function Review({ state }: { state: Extract<ImportState, { phase: "review" | "importing" }> }) {
  const { scan } = state;
  const importing = state.phase === "importing";
  const chosen = state.phase === "review" ? state.chosen : null;
  const { toggleChosen, runImport, dismissImport } = useStore.getState();

  const groups = useMemo(() => {
    const by = (status: ScanItem["status"]) => scan.items.filter((item) => item.status === status);
    return { fresh: by("new"), deleted: by("deleted"), duplicate: by("duplicate") };
  }, [scan]);

  const sections = useMemo(() => {
    const list: GridSection<ScanItem>[] = [];
    const add = (key: string, items: ScanItem[], title: string, detail?: string) => {
      if (items.length > 0) list.push({ key, items, header: <SectionHeading title={title} detail={detail} /> });
    };
    add("new", groups.fresh, `${count(groups.fresh.length)} new`);
    add(
      "deleted",
      groups.deleted,
      `${count(groups.deleted.length)} in Recently Deleted`,
      "Importing these puts them back in your library",
    );
    add("duplicate", groups.duplicate, `${count(groups.duplicate.length)} already in your library`);
    return list;
  }, [groups]);

  const selectable = [...groups.fresh, ...groups.deleted].map((item) => item.index);
  const chosenCount = chosen?.size ?? 0;
  const progress = importing ? state.progress : null;

  return (
    <div className="panel sheet">
      <header className="sheet-header">
        <div className="toolbar-title">
          <h1>Import from {scan.source}</h1>
          <span>
            {selectable.length === 0
              ? "Everything here is already in your library"
              : "Choose what to copy into your library. The originals stay where they are."}
          </span>
        </div>
        {!importing && selectable.length > 0 && (
          <div className="toolbar-controls">
            <button className="button" onClick={() => toggleChosen(selectable, true)}>
              Select all
            </button>
            <button className="button" onClick={() => toggleChosen(selectable, false)}>
              Select none
            </button>
          </div>
        )}
      </header>

      <VirtualGrid
        className={`sheet-grid ${importing ? "inert" : ""}`}
        sections={sections}
        cellSize={150}
        itemKey={itemKey}
        renderCell={(item) => (
          <Candidate
            item={item}
            sessionId={scan.sessionId}
            chosen={chosen?.has(item.index) ?? false}
            locked={importing}
          />
        )}
      />

      <footer className="sheet-footer">
        {progress ? (
          <>
            <div className="progress" role="progressbar" aria-valuemin={0} aria-valuemax={progress.total} aria-valuenow={progress.done}>
              <div style={{ width: `${(progress.done / Math.max(1, progress.total)) * 100}%` }} />
            </div>
            <span className="progress-label">
              Copying {count(progress.done)} of {count(progress.total)}
            </span>
            <button className="button" onClick={() => void dismissImport()}>
              Stop
            </button>
          </>
        ) : (
          <>
            <span className="progress-label">{plural(chosenCount, "photo")} selected</span>
            <button className="button" onClick={() => void dismissImport()}>
              Cancel
            </button>
            <button className="button primary" disabled={chosenCount === 0} onClick={() => void runImport()}>
              Import {plural(chosenCount, "photo")}
            </button>
          </>
        )}
      </footer>
    </div>
  );
}

/** The import flow: looking for photos, reviewing them, copying, and any failures. */
export function ImportSheet() {
  const state = useStore((s) => s.importState);

  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      const current = useStore.getState();
      if (current.confirmRequest || current.importState?.phase === "importing") return;
      if (event.key === "Escape") void current.dismissImport();
      // Enter is the sheet's main action unless a button has the focus.
      if (event.key === "Enter" && !(event.target as HTMLElement).closest("button")) void current.runImport();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  if (!state) return null;
  return (
    <div className="scrim">
      {state.phase === "scanning" && (
        <div className="panel dialog" role="status">
          <h2>Looking for photos…</h2>
          <p>
            {state.progress && state.progress.total > 0
              ? `Checked ${count(state.progress.done)} of ${count(state.progress.total)} against your library`
              : "Reading folders"}
          </p>
          {/* Counting can't start until the folders have been read; until then the bar just keeps moving. */}
          <div className="dialog-progress">
            {state.progress && state.progress.total > 0 ? (
              <div className="progress" role="progressbar" aria-valuemin={0} aria-valuemax={state.progress.total} aria-valuenow={state.progress.done}>
                <div style={{ width: `${(state.progress.done / state.progress.total) * 100}%` }} />
              </div>
            ) : (
              <div className="progress waiting" role="progressbar" aria-label="Reading folders">
                <div />
              </div>
            )}
          </div>
        </div>
      )}
      {(state.phase === "review" || state.phase === "importing") && <Review state={state} />}
      {state.phase === "done" && (
        <div className="panel dialog" role="alertdialog">
          <h2>{plural(state.summary.failed.length, "photo")} couldn’t be imported</h2>
          <p>
            {state.summary.imported + state.summary.restored > 0
              ? `The other ${plural(state.summary.imported + state.summary.restored, "photo")} are in your library.`
              : "Nothing was added to your library."}
          </p>
          <ul className="failures">
            {state.summary.failed.map((failure) => (
              <li key={failure.fileName}>
                <strong>{failure.fileName}</strong>
                <span>{failure.reason}</span>
              </li>
            ))}
          </ul>
          <div className="dialog-actions">
            <button className="button primary" onClick={() => void useStore.getState().dismissImport()}>
              Done
            </button>
          </div>
        </div>
      )}
    </div>
  );
}
