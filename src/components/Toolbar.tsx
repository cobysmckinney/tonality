import { useEffect, useId, useRef, useState } from "react";
import { BookImage, Check, ChevronDown, Crop, Download, Heart, ListFilter, Search, Trash2, Undo2, X } from "lucide-react";
import type { Overview, View } from "../api";
import { count, plural } from "../format";
import { albumEntries } from "../menus";
import { applySuggestion, KIND_LABELS, searching, SORTS, suggest } from "../search";
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

/** How many photos a view holds, searched for or not. */
function viewTotal(overview: Overview | null, view: View): number | null {
  if (!overview) return null;
  switch (view.kind) {
    case "library":
    case "imports":
      return overview.photoCount;
    case "favorites":
      return overview.favoriteCount;
    case "deleted":
      return overview.deletedCount;
    case "album":
      return overview.albums.find((a) => a.id === view.id)?.count ?? null;
  }
}

/**
 * The search field. As it is typed, the grid keeps to the photos that
 * match, and the cameras, lenses, film stocks and dates in view are offered
 * under it; with nothing typed, a few of each, to show what can be found.
 */
function SearchField() {
  const search = useStore((s) => s.search);
  const facets = useStore((s) => s.facets);
  const [open, setOpen] = useState(false);
  const [active, setActive] = useState(-1);
  const field = useRef<HTMLInputElement>(null);
  const list = useId();
  const shown = open && facets ? suggest(search, facets) : [];
  const { setSearch, loadFacets } = useStore.getState();

  const choose = (index: number) => {
    setSearch(applySuggestion(search, shown[index]), true);
    setActive(-1);
  };

  // Ctrl+F from anywhere in the library.
  useEffect(() => {
    const onKey = (event: KeyboardEvent) => {
      if (event.key.toLowerCase() !== "f" || !(event.ctrlKey || event.metaKey) || event.shiftKey || event.altKey) return;
      const s = useStore.getState();
      if (s.openId !== null || s.importState || s.exportState || s.filmSheet || s.confirmRequest || s.menu || s.shortcutsOpen) {
        return;
      }
      event.preventDefault();
      field.current?.focus();
      field.current?.select();
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  // What there is to find changes with the view, and with film details saved.
  useEffect(() => {
    if (open && facets === null) void loadFacets();
  }, [open, facets, loadFacets]);

  return (
    <div className={`search ${searching(search) ? "engaged" : ""}`}>
      <Search size={14} className="search-icon" />
      <input
        ref={field}
        className="search-input"
        value={search}
        placeholder="Search"
        title="Search by camera, lens, film or date (Ctrl+F)"
        aria-label="Search by camera, lens, film or date"
        role="combobox"
        aria-expanded={shown.length > 0}
        aria-controls={list}
        aria-autocomplete="list"
        aria-activedescendant={active >= 0 ? `${list}-${active}` : undefined}
        spellCheck={false}
        autoComplete="off"
        onFocus={() => setOpen(true)}
        onMouseDown={() => setOpen(true)}
        onBlur={() => {
          setOpen(false);
          setActive(-1);
        }}
        onChange={(event) => {
          setSearch(event.currentTarget.value);
          setOpen(true);
          setActive(-1);
        }}
        onKeyDown={(event) => {
          if (event.key === "ArrowDown" || event.key === "ArrowUp") {
            event.preventDefault();
            if (!open) {
              setOpen(true);
              return;
            }
            const last = shown.length - 1;
            if (event.key === "ArrowDown") setActive((at) => (at >= last ? 0 : at + 1));
            else setActive((at) => (at <= 0 ? last : at - 1));
          } else if (event.key === "Enter") {
            event.preventDefault();
            if (active >= 0 && active < shown.length) choose(active);
            else {
              setSearch(search, true);
              setOpen(false);
            }
          } else if (event.key === "Escape") {
            event.preventDefault();
            // One thing at a time: the suggestions, then what was typed, then the field.
            if (shown.length > 0) setOpen(false);
            else if (search !== "") setSearch("", true);
            else event.currentTarget.blur();
            setActive(-1);
          }
        }}
      />
      {search !== "" && (
        <button
          className="search-clear"
          aria-label="Clear the search"
          title="Clear the search"
          // Keeps the focus in the field.
          onMouseDown={(event) => event.preventDefault()}
          onClick={() => setSearch("", true)}
        >
          <X size={13} />
        </button>
      )}
      {shown.length > 0 && (
        <ul className="suggestions search-suggestions" id={list} role="listbox" aria-label="Suggestions">
          {shown.map((suggestion, i) => (
            <li
              key={suggestion.insert}
              id={`${list}-${i}`}
              role="option"
              aria-selected={i === active}
              className={i === active ? "active" : ""}
              // Before the field loses the focus, which would close the list.
              onMouseDown={(event) => {
                event.preventDefault();
                choose(i);
              }}
            >
              <span className="suggestion-label">{suggestion.label}</span>
              <span className="suggestion-kind">{KIND_LABELS[suggestion.kind]}</span>
              <span className="suggestion-count">{count(suggestion.count)}</span>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
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
  const listed = useStore((s) => s.photos.length);
  const shown = useStore((s) => visiblePhotos(s).length);
  const filter = useStore((s) => s.filter);
  const sort = useStore((s) => s.sort);
  const searched = useStore((s) => searching(s.search));
  const inView = useStore((s) => viewTotal(s.overview, s.view));
  const thumbSize = useStore((s) => s.thumbSize);
  const square = useStore((s) => s.squareThumbs);
  const selection = useStore((s) => s.selection);
  const s = useStore.getState();
  const inTrash = view.kind === "deleted";
  // The view's own order wins in Imports and Recently Deleted.
  const sortable = view.kind !== "imports" && !inTrash;
  const narrowed = searched || (filter !== "all" && !inTrash);
  const total = searched ? Math.max(inView ?? listed, listed) : listed;

  const counted = narrowed ? `${plural(shown, "photo")} of ${plural(total, "photo")}` : plural(total, "photo");
  const summary = inTrash ? (total > 0 ? `${counted}, kept for 30 days` : "") : counted;

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
        <SearchField />
        {inTrash ? (
          listed > 0 && (
            <button className="button danger" onClick={() => void s.purge(useStore.getState().photos.map((p) => p.id))}>
              {searched ? "Delete these" : "Delete all"}
            </button>
          )
        ) : (
          <button
            className={`button ${filter !== "all" ? "engaged" : ""}`}
            title="Filter and sort"
            onClick={(event) =>
              menuBelow(event, [
                ...FILTERS.map((f) => ({
                  label: f.label,
                  checked: f.value === filter,
                  run: () => s.setFilter(f.value),
                })),
                "separator",
                ...(sortable
                  ? [
                      ...SORTS.map((order) => ({
                        label: order.label,
                        checked: order.value === sort,
                        run: () => s.setSort(order.value),
                      })),
                      "separator" as const,
                    ]
                  : []),
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
