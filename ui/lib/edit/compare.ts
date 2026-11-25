// What Edit shows side by side with the photo being edited: another version of it, or another
// photo (a reference, to match a look across a set).
import type { EditState } from "$lib/bindings/EditState";
import type { HistoryView } from "$lib/bindings/HistoryView";
import type { ImageInfo } from "$lib/bindings/ImageInfo";

/** Another version (branch tip) of the open photo, shown on the right. */
export interface VersionCompare {
  kind: "version";
  branch: string;
  state: EditState;
}

/** Another photo, read-only, at its own current version, shown on the left. */
export interface PhotoCompare {
  kind: "photo";
  image: ImageInfo;
  /** Its preview pixels (RGB16F), as `previewPixels` returns them. */
  pixels: Uint16Array;
  /** The version it shows. */
  version: string;
  state: EditState;
  /** Not a version of the open photo: its version rows don't light up. */
  branch?: never;
}

export type Compare = VersionCompare | PhotoCompare;

/** What's left of the compare when photo `id` opens: a version belongs to the photo it came
 * from; a reference stays, unless it's the photo being opened. */
export function afterOpen(c: Compare | null, id: number): Compare | null {
  if (!c || c.kind === "version") return null;
  return c.image.id === id ? null : c;
}

/** What's left once the open photo's history changed to `view`: a version that was deleted,
 * or is now the one being edited, stops. A reference isn't affected. */
export function afterHistory(c: Compare | null, view: HistoryView): Compare | null {
  if (!c || c.kind === "photo") return c;
  const gone = !view.branches.some((b) => b.name === c.branch);
  return gone || c.branch === view.head.branch ? null : c;
}

/** What's left once photos `ids` were removed from the library. */
export function afterRemoved(c: Compare | null, ids: number[]): Compare | null {
  return c?.kind === "photo" && ids.includes(c.image.id) ? null : c;
}

/** The pane labels: the photo being edited (`main`) and what it's compared with (`other`).
 * `current` is the version being edited, `name` the open photo's file name. */
export function compareLabels(
  c: Compare,
  current: string,
  name: string,
): { main: string; other: string } {
  if (c.kind === "version") return { main: current, other: c.branch };
  return { main: name, other: `${c.image.name} · ${c.version}` };
}
