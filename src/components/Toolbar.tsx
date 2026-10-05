import { BookImage, Check, ChevronDown, Crop, Download, Heart, ListFilter, Trash2, Undo2, X } from "lucide-react";
import { count, plural } from "../format";
import { albumEntries } from "../menus";
import { Filter, MenuEntry, useStore, visiblePhotos } from "../store";

const FILTERS: { value: Filter; label: string }[] = [
  { value: "all", label: "All photos" },
  { value: "picks", label: "Picks only" },
  { value: "unrejected", label: "Hide rejected" },
  { value: "rejects", label: "Rejected only" },
];

/** Opens a menu hanging from the button that was clicked. */
export function menuBelow(event: React.MouseEvent, entries: MenuEntry[]) {
  const rect = event.currentTarget.getBoundingClientRect();
  useStore.getState().openMenu(rect.left, rect.bottom + 6, entries);
}

export function importEntries(): MenuEntry[] {
  const { importFiles, importFolder, startImport, volumes } = useStore.getState();
  return [
    { label: "Choose files…", run: () => void importFiles() },
    { label: "Choose a folder…", run: () => void importFolder() },
    ...(volumes.length > 0 ? ["separator" as const] : []),
    ...volumes.map((card) => ({
      label: `From “${card.name}”`,
      run: () => void startImport([card.path], card.name),
    })),
  ];
}

export function useTitle(): string {
  const view = useStore((s) => s.view);
  const albums = useStore((s) => s.overview?.albums);
  switch (view.kind) {
    case "library":
      return "Library";
    case "favorites":
      return "Favorites";
    case "imports":
      return "Imports";
    case "deleted":
      return "Recently Deleted";
    case "album":
      return albums?.find((a) => a.id === view.id)?.name ?? "Album";
  }
}

function SelectionActions({ ids }: { ids: number[] }) {
  const inTrash = useStore((s) => s.view.kind === "deleted");
  const s = useStore.getState();

  return (
    <div className="selection-actions">
      <span className="selection-count">{count(ids.length)} selected</span>
      {inTrash ? (
        <>
          <button className="button" onClick={() => void s.restore(ids)}>
            <Undo2 size={15} /> Recover
          </button>
          <button className="button danger" onClick={() => void s.purge(ids)}>
            Delete permanently
          </button>
        </>
      ) : (
        <>
          <button className="icon-button" title="Favorite (F)" aria-label="Favorite" onClick={() => void s.toggleFavorite(ids)}>
            <Heart size={16} />
          </button>
          <button className="icon-button" title="Pick (P)" aria-label="Pick" onClick={() => void s.toggleFlag(ids, 1)}>
            <Check size={16} />
          </button>
          <button className="icon-button" title="Reject (X)" aria-label="Reject" onClick={() => void s.toggleFlag(ids, -1)}>
            <X size={16} />
          </button>
          <button
            className="icon-button"
            title="Add to album"
            aria-label="Add to album"
            onClick={(event) => menuBelow(event, albumEntries(ids))}
          >
            <BookImage size={16} />
          </button>
          <button className="icon-button" title="Delete (Del)" aria-label="Delete" onClick={() => void s.trash(ids)}>
            <Trash2 size={16} />
          </button>
          <button className="button" title="Export (Ctrl+E)" onClick={() => void s.startExport(ids)}>
            <Download size={15} /> <span className="collapsible">Export</span>
          </button>
        </>
      )}
    </div>
  );
}

export function Toolbar() {
  const title = useTitle();
  const view = useStore((s) => s.view);
  const total = useStore((s) => s.photos.length);
  const shown = useStore((s) => visiblePhotos(s).length);
  const filter = useStore((s) => s.filter);
  const thumbSize = useStore((s) => s.thumbSize);
  const square = useStore((s) => s.squareThumbs);
  const selection = useStore((s) => s.selection);
  const s = useStore.getState();
  const inTrash = view.kind === "deleted";

  const summary = inTrash
    ? total > 0
      ? `${plural(total, "photo")}, kept for 30 days`
      : ""
    : filter === "all"
      ? plural(total, "photo")
      : `${plural(shown, "photo")} of ${plural(total, "photo")}`;

  return (
    <header className="toolbar">
      {/* A selection takes over the title's place, so its actions are always in reach. */}
      {selection.size > 0 ? (
        <SelectionActions ids={[...selection]} />
      ) : (
        <div className="toolbar-title">
          <h1>{title}</h1>
          <span>{summary}</span>
        </div>
      )}

      <div className="toolbar-controls">
        {inTrash ? (
          total > 0 && (
            <button className="button danger" onClick={() => void s.purge(useStore.getState().photos.map((p) => p.id))}>
              Delete all
            </button>
          )
        ) : (
          <button
            className={`button ${filter !== "all" ? "engaged" : ""}`}
            onClick={(event) =>
              menuBelow(event, [
                ...FILTERS.map((f) => ({
                  label: f.label,
                  checked: f.value === filter,
                  run: () => s.setFilter(f.value),
                })),
                "separator",
                { label: "Delete rejected photos", danger: true, run: () => void s.trashRejected() },
              ])
            }
          >
            <ListFilter size={15} />
            <span className="collapsible">{FILTERS.find((f) => f.value === filter)!.label}</span>
            <ChevronDown size={14} />
          </button>
        )}

        <button
          className={`icon-button ${square ? "" : "engaged"}`}
          title={square ? "Show whole photos" : "Fill squares"}
          aria-label="Toggle square thumbnails"
          onClick={s.toggleSquareThumbs}
        >
          <Crop size={16} />
        </button>
        <input
          className="zoom"
          type="range"
          min={96}
          max={360}
          step={4}
          value={thumbSize}
          aria-label="Thumbnail size"
          onChange={(event) => s.setThumbSize(Number(event.currentTarget.value))}
        />

        <button className="button primary" onClick={(event) => menuBelow(event, importEntries())}>
          Import
          <ChevronDown size={14} />
        </button>
      </div>
    </header>
  );
}
