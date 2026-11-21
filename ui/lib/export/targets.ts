// What "Export" applies to: the photo in Edit, or the Library selection (else the focused photo).
import { app } from "$lib/app.svelte";
import { session } from "$lib/edit/session.svelte";
import { library } from "$lib/library/state.svelte";
import { exporter } from "./export.svelte";

function targets(): number[] {
  if (app.mode === "edit") return session.image ? [session.image.id] : [];
  if (library.selected.size) return [...library.selected];
  return library.focus !== null ? [library.focus] : [];
}

/** Number of photos Export would apply to (reactive). */
export const exportCount = () => targets().length;

export function exportCurrent() {
  exporter.open(targets());
}
