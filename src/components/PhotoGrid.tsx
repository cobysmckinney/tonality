import { memo, useEffect, useMemo, useRef, useState } from "react";
import { Check, FileX, GitBranch, Heart, ImageOff, SlidersHorizontal, X } from "lucide-react";
import { Photo, thumbUrl } from "../api";
import { count, dayHeading, daysLeft, longDateTime, plural } from "../format";
import { photoMenu } from "../menus";
import { targetOf, useStore, visiblePhotos } from "../store";
import { Direction, GridHandle, GridSection, VirtualGrid } from "./VirtualGrid";

const photoKey = (photo: Photo) => photo.id;

function Heading({ title, detail, total }: { title: string; detail: string; total: number }) {
  return (
    <header className="day-heading">
      <h2>{title}</h2>
      <span className="day-detail">{detail}</span>
      <span className="day-count">{count(total)}</span>
    </header>
  );
}

const PhotoCell = memo(function PhotoCell(props: {
  photo: Photo;
  selected: boolean;
  square: boolean;
  inTrash: boolean;
}) {
  const { photo, selected, square, inTrash } = props;
  const [failed, setFailed] = useState(false);
  const classes = ["photo", selected && "selected", photo.flag === -1 && "rejected", !square && "fit"];

  return (
    <div
      className={classes.filter(Boolean).join(" ")}
      title={photo.missing ? `${photo.fileName}: the original file is missing from the library folder` : photo.fileName}
      onClick={(event) => {
        useStore.getState().click(photo.id, { range: event.shiftKey, toggle: event.ctrlKey || event.metaKey });
      }}
      onDoubleClick={() => !inTrash && useStore.getState().openPhoto(photo.id)}
      onContextMenu={(event) => {
        event.preventDefault();
        const state = useStore.getState();
        if (!state.selection.has(photo.id)) state.click(photo.id, { range: false, toggle: false });
        state.openMenu(event.clientX, event.clientY, photoMenu(targetOf(useStore.getState(), photo.id)));
      }}
    >
      {failed ? (
        <div className="photo-missing">
          <ImageOff size={20} />
          <span>{photo.fileName}</span>
        </div>
      ) : (
        <img src={thumbUrl(photo)} alt="" draggable={false} decoding="async" onError={() => setFailed(true)} />
      )}

      {photo.flag === 1 && (
        <span className="badge flag pick" aria-label="Pick">
          <Check size={12} strokeWidth={3} />
        </span>
      )}
      {photo.flag === -1 && (
        <span className="badge flag reject" aria-label="Rejected">
          <X size={12} strokeWidth={3} />
        </span>
      )}
      <span className="badge marks">
        {photo.missing && (
          <span className="mark" aria-label="Original missing">
            <FileX size={11} strokeWidth={2.5} />
          </span>
        )}
        {photo.branches > 1 && (
          <span className="mark" title={`${photo.branches} branches of edits; showing the current one`}>
            <GitBranch size={11} strokeWidth={2.5} />
            {photo.branches}
          </span>
        )}
        {photo.edited && (
          <span className="mark" aria-label="Edited">
            <SlidersHorizontal size={11} strokeWidth={2.5} />
          </span>
        )}
      </span>
      {photo.hasJpeg && <span className="badge pair">RAW + JPEG</span>}
      {inTrash && photo.deletedAt !== null && (
        <span className="badge pair">{plural(daysLeft(photo.deletedAt), "day")} left</span>
      )}

      {!inTrash && (
        <button
          className={`heart ${photo.favorite ? "on" : ""}`}
          aria-label={photo.favorite ? "Remove from Favorites" : "Add to Favorites"}
          onClick={(event) => {
            event.stopPropagation();
            void useStore.getState().toggleFavorite([photo.id]);
          }}
          onDoubleClick={(event) => event.stopPropagation()}
        >
          <Heart size={15} fill={photo.favorite ? "currentColor" : "none"} />
        </button>
      )}
    </div>
  );
});

/** The main grid: the current view's photos under day (or import) headings. */
export function PhotoGrid() {
  const view = useStore((s) => s.view);
  const photos = useStore(visiblePhotos);
  const imports = useStore((s) => s.overview?.imports);
  const selection = useStore((s) => s.selection);
  const thumbSize = useStore((s) => s.thumbSize);
  const square = useStore((s) => s.squareThumbs);
  const grid = useRef<GridHandle<number>>(null);

  const sections = useMemo(() => {
    if (view.kind === "deleted") return [{ key: "all", items: photos }];
    const groups: GridSection<Photo>[] = [];
    const keyOf = (photo: Photo) => (view.kind === "imports" ? String(photo.importId) : photo.takenAt.slice(0, 10));
    // Photos arrive sorted, so each group is one contiguous run.
    for (const photo of photos) {
      const key = keyOf(photo);
      if (groups.at(-1)?.key !== key) groups.push({ key, items: [] });
      groups.at(-1)!.items.push(photo);
    }
    for (const group of groups) {
      if (view.kind === "imports") {
        const info = imports?.find((i) => String(i.id) === group.key);
        const when = info ? longDateTime(new Date(info.createdAt * 1000)) : "";
        group.header = <Heading title={info?.source ?? "Import"} detail={when} total={group.items.length} />;
      } else {
        group.header = <Heading {...dayHeading(group.key)} total={group.items.length} />;
      }
    }
    return groups;
  }, [photos, view, imports]);

  // Coming back from the viewer, bring the photo that was open into view.
  const viewing = useStore((s) => s.openId !== null);
  useEffect(() => {
    const cursor = useStore.getState().cursor;
    if (!viewing && cursor !== null) grid.current?.reveal(cursor);
  }, [viewing]);

  // Keyboard: arrows move, with Shift they extend; letters mark; Enter opens.
  useEffect(() => {
    const arrows: Record<string, Direction> = {
      ArrowLeft: "left",
      ArrowRight: "right",
      ArrowUp: "up",
      ArrowDown: "down",
    };
    const onKey = (event: KeyboardEvent) => {
      const s = useStore.getState();
      const typing = (event.target as HTMLElement).closest("input, textarea");
      if (typing || s.openId !== null || s.importState || s.exportState || s.confirmRequest || s.menu) return;
      const selected = [...s.selection];
      const inTrash = s.view.kind === "deleted";
      const key = event.key.length === 1 ? event.key.toLowerCase() : event.key;

      if (key in arrows) {
        event.preventDefault();
        const from = s.cursor !== null && s.selection.has(s.cursor) ? s.cursor : null;
        const next = from === null ? grid.current?.first() : grid.current?.neighbor(from, arrows[key]);
        if (next != null) {
          s.moveCursor(next, event.shiftKey && from !== null);
          grid.current?.reveal(next);
        }
      } else if (key === "a" && (event.ctrlKey || event.metaKey)) {
        event.preventDefault();
        s.selectAll();
      } else if (key === "c" && (event.ctrlKey || event.metaKey) && selected.length === 1 && !inTrash) {
        void s.copyEdits(selected[0]);
      } else if (key === "v" && (event.ctrlKey || event.metaKey) && selected.length > 0 && !inTrash) {
        void s.pasteEdits(selected);
      } else if (key === "e" && (event.ctrlKey || event.metaKey) && selected.length > 0 && !inTrash) {
        event.preventDefault();
        void s.startExport(selected);
      } else if (event.ctrlKey || event.metaKey || event.altKey) {
        return;
      } else if (key === "Escape") {
        s.clearSelection();
      } else if (selected.length === 0) {
        return;
      } else if ((event.target as HTMLElement).closest("button") && (key === "Enter" || key === " ")) {
        // Let a focused button take its own activation keys.
        return;
      } else if ((key === "Enter" || key === " ") && !inTrash) {
        event.preventDefault();
        s.openPhoto(s.cursor !== null && s.selection.has(s.cursor) ? s.cursor : selected[0]);
      } else if (key === "Delete" || key === "Backspace") {
        void (inTrash ? s.purge(selected) : s.trash(selected));
      } else if (inTrash) {
        return;
      } else if (key === "f") void s.toggleFavorite(selected);
      else if (key === "p") void s.toggleFlag(selected, 1);
      else if (key === "x") void s.toggleFlag(selected, -1);
      else if (key === "u") void s.toggleFlag(selected, 0);
    };
    window.addEventListener("keydown", onKey);
    return () => window.removeEventListener("keydown", onKey);
  }, []);

  const inTrash = view.kind === "deleted";
  return (
    <VirtualGrid
      className="photo-grid"
      sections={sections}
      cellSize={thumbSize}
      itemKey={photoKey}
      handle={grid}
      onBackgroundClick={() => useStore.getState().clearSelection()}
      renderCell={(photo) => (
        <PhotoCell photo={photo} selected={selection.has(photo.id)} square={square} inTrash={inTrash} />
      )}
    />
  );
}
