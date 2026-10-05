import { ReactNode, useEffect, useRef } from "react";
import { BookImage, Camera, Clock, Heart, Images, Plus, Trash2 } from "lucide-react";
import { Album, View } from "../api";
import { count } from "../format";
import { useStore } from "../store";

function Item(props: {
  icon: ReactNode;
  label: ReactNode;
  total?: number;
  active?: boolean;
  onClick: () => void;
  onContextMenu?: (event: React.MouseEvent) => void;
  onDoubleClick?: () => void;
}) {
  return (
    <button
      className={`side-item ${props.active ? "active" : ""}`}
      onClick={props.onClick}
      onContextMenu={props.onContextMenu}
      onDoubleClick={props.onDoubleClick}
    >
      {props.icon}
      <span className="side-label">{props.label}</span>
      {props.total !== undefined && props.total > 0 && <span className="side-count">{count(props.total)}</span>}
    </button>
  );
}

function AlbumName({ album }: { album: Album }) {
  const input = useRef<HTMLInputElement>(null);
  useEffect(() => input.current?.select(), []);
  return (
    <div className="side-item editing">
      <BookImage size={16} />
      <input
        ref={input}
        defaultValue={album.name}
        aria-label="Album name"
        onBlur={(event) => void useStore.getState().renameAlbum(album.id, event.currentTarget.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") event.currentTarget.blur();
          if (event.key === "Escape") useStore.getState().setRenamingAlbum(null);
        }}
      />
    </div>
  );
}

export function Sidebar() {
  const overview = useStore((s) => s.overview);
  const view = useStore((s) => s.view);
  const volumes = useStore((s) => s.volumes);
  const renaming = useStore((s) => s.renamingAlbum);
  const { setView, newAlbum, setRenamingAlbum, deleteAlbum, openMenu, startImport } = useStore.getState();
  const go = (next: View) => () => void setView(next);

  return (
    <nav className="panel sidebar">
      <div className="side-group">
        <Item
          icon={<Images size={16} />}
          label="Library"
          total={overview?.photoCount}
          active={view.kind === "library"}
          onClick={go({ kind: "library" })}
        />
        <Item
          icon={<Heart size={16} />}
          label="Favorites"
          total={overview?.favoriteCount}
          active={view.kind === "favorites"}
          onClick={go({ kind: "favorites" })}
        />
        <Item
          icon={<Clock size={16} />}
          label="Imports"
          active={view.kind === "imports"}
          onClick={go({ kind: "imports" })}
        />
        <Item
          icon={<Trash2 size={16} />}
          label="Recently Deleted"
          total={overview?.deletedCount}
          active={view.kind === "deleted"}
          onClick={go({ kind: "deleted" })}
        />
      </div>

      <div className="side-group">
        <div className="side-heading">
          <span>Albums</span>
          <button className="icon-button small" aria-label="New album" title="New album" onClick={() => void newAlbum([])}>
            <Plus size={14} />
          </button>
        </div>
        {overview?.albums.map((album) =>
          renaming === album.id ? (
            <AlbumName key={album.id} album={album} />
          ) : (
            <Item
              key={album.id}
              icon={<BookImage size={16} />}
              label={album.name}
              total={album.count}
              active={view.kind === "album" && view.id === album.id}
              onClick={go({ kind: "album", id: album.id })}
              onDoubleClick={() => setRenamingAlbum(album.id)}
              onContextMenu={(event) => {
                event.preventDefault();
                openMenu(event.clientX, event.clientY, [
                  { label: "Rename", run: () => setRenamingAlbum(album.id) },
                  { label: "Delete album", danger: true, run: () => void deleteAlbum(album.id) },
                ]);
              }}
            />
          ),
        )}
        {overview?.albums.length === 0 && <p className="side-hint">Group photos any way you like.</p>}
      </div>

      {volumes.length > 0 && (
        <div className="side-group">
          <div className="side-heading">
            <span>Camera cards</span>
          </div>
          {volumes.map((card) => (
            <Item
              key={card.path}
              icon={<Camera size={16} />}
              label={card.name}
              onClick={() => void startImport([card.path], card.name)}
            />
          ))}
        </div>
      )}
    </nav>
  );
}
