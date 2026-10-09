/**
 * The import, export and film details sheets, and when one may open. Only
 * one sheet is ever open: each would otherwise answer Esc and Enter at the
 * same time.
 */

interface Open {
  importState: { phase: string } | null;
  exportState: unknown;
  filmSheet: unknown;
  confirmRequest: unknown;
}

/** Whether an import may start: not over another sheet or a question, nor while one is already looking or copying. */
export const canStartImport = (s: Open) =>
  !s.exportState &&
  !s.filmSheet &&
  !s.confirmRequest &&
  s.importState?.phase !== "scanning" &&
  s.importState?.phase !== "importing";

/** Whether an export may start: not over another sheet, nor while one is already open. */
export const canStartExport = (s: Open) => !s.exportState && !s.importState && !s.filmSheet;

/** Whether the film details sheet may open: not over another sheet or a question. */
export const canStartFilm = (s: Open) => !s.exportState && !s.importState && !s.filmSheet && !s.confirmRequest;
