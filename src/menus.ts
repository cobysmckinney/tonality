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

/** Every preset, to apply to a set of photos; a line parts the groups. */
export function presetEntries(ids: number[]): MenuEntry[] {
  const { presets, applyPreset } = useStore.getState();
  return presets.flatMap((preset, i): MenuEntry[] => [
    ...(i > 0 && presets[i - 1].group !== preset.group ? ["separator" as const] : []),
    { label: preset.name, run: () => void applyPreset(ids, preset) },
  ]);
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
      { label: "Delete permanently", hint: "Del", danger: true, run: () => void s.purge(ids) },
    ];
  }

  const view = s.view;
  return [
    ...(single !== null ? [{ label: "Open", hint: "Enter", run: () => s.openPhoto(single) }, "separator" as const] : []),
    {
      label: photos.every((p) => p.favorite) ? "Remove from Favorites" : "Add to Favorites",
      hint: "F",
      run: () => void s.toggleFavorite(ids),
    },
    { label: "Pick", hint: "P", checked: photos.every((p) => p.flag === 1), run: () => void s.toggleFlag(ids, 1) },
    { label: "Reject", hint: "X", checked: photos.every((p) => p.flag === -1), run: () => void s.toggleFlag(ids, -1) },
    "separator",
    ...(single !== null ? [{ label: "Copy edits", hint: "Ctrl+C", run: () => void s.copyEdits(single) }] : []),
    { label: "Paste edits", hint: "Ctrl+V", disabled: s.clipboard === null, run: () => void s.pasteEdits(ids) },
    { label: "Apply preset", submenu: presetEntries(ids) },
    // Frames of one roll of film, measured together so they match and keep their own colours.
    ...(ids.length > 1 ? [{ label: "Balance as one roll", run: () => void s.balanceRoll(ids) }] : []),
    { label: "Revert to original", disabled: !photos.some((p) => p.edited), run: () => void s.revertEdits(ids) },
    "separator",
    { label: "Export…", hint: "Ctrl+E", run: () => void s.startExport(ids) },
    "separator",
    { label: "Add to album", submenu: albumEntries(ids) },
    ...(view.kind === "album"
      ? [{ label: "Remove from this album", run: () => void s.removeFromAlbum(view.id, ids) }]
      : []),
    { label: "Film details…", run: () => void s.editFilmDetails(ids) },
    ...(single !== null
      ? [
          {
            label: "Show in file manager",
            run: () => void api.getPhotoInfo(single).then((info) => s.reveal(info.path)),
          },
        ]
      : []),
    "separator",
    { label: "Delete", hint: "Del", danger: true, run: () => void s.trash(ids) },
  ];
}
