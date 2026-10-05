import { revealItemInDir } from "@tauri-apps/plugin-opener";
import { api } from "./api";
import { MenuEntry, useStore } from "./store";

/** "Add to album" choices for a set of photos. */
export function albumEntries(ids: number[]): MenuEntry[] {
  const { overview, addToAlbum, newAlbum } = useStore.getState();
  const albums = overview?.albums ?? [];
  return [
    ...albums.map((album) => ({ label: album.name, run: () => void addToAlbum(album.id, ids) })),
    ...(albums.length > 0 ? ["separator" as const] : []),
    { label: "New album", run: () => void newAlbum(ids) },
  ];
}

/** The right-click menu for one or more photos in the current view. */
export function photoMenu(ids: number[]): MenuEntry[] {
  const s = useStore.getState();
  const photos = s.photos.filter((p) => ids.includes(p.id));
  const single = ids.length === 1 ? ids[0] : null;

  if (s.view.kind === "deleted") {
    return [
      { label: "Recover", run: () => void s.restore(ids) },
      "separator",
      { label: "Delete permanently", danger: true, run: () => void s.purge(ids) },
    ];
  }

  const view = s.view;
  return [
    ...(single !== null ? [{ label: "Open", run: () => s.openPhoto(single) }, "separator" as const] : []),
    {
      label: photos.every((p) => p.favorite) ? "Remove from Favorites" : "Add to Favorites",
      run: () => void s.toggleFavorite(ids),
    },
    { label: "Pick", checked: photos.every((p) => p.flag === 1), run: () => void s.toggleFlag(ids, 1) },
    { label: "Reject", checked: photos.every((p) => p.flag === -1), run: () => void s.toggleFlag(ids, -1) },
    "separator",
    ...(single !== null ? [{ label: "Copy edits", run: () => void s.copyEdits(single) }] : []),
    { label: "Paste edits", disabled: s.clipboard === null, run: () => void s.pasteEdits(ids) },
    { label: "Revert to original", disabled: !photos.some((p) => p.edited), run: () => void s.revertEdits(ids) },
    "separator",
    { label: "Export…", run: () => void s.startExport(ids) },
    "separator",
    { label: "Add to album", submenu: albumEntries(ids) },
    ...(view.kind === "album"
      ? [{ label: "Remove from this album", run: () => void s.removeFromAlbum(view.id, ids) }]
      : []),
    ...(single !== null
      ? [
          {
            label: "Show in file manager",
            run: () => void api.getPhotoInfo(single).then((info) => revealItemInDir(info.path)),
          },
        ]
      : []),
    "separator",
    { label: "Delete", danger: true, run: () => void s.trash(ids) },
  ];
}
