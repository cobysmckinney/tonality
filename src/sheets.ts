/**
 * The import and export sheets, and when one may open. Only one sheet is ever
 * open: each would otherwise answer Esc and Enter at the same time.
 */

interface Open {
  importState: { phase: string } | null;
  exportState: unknown;
  confirmRequest: unknown;
}

/** Whether an import may start: not over the export sheet or a question, nor while one is already looking or copying. */
export const canStartImport = (s: Open) =>
  !s.exportState && !s.confirmRequest && s.importState?.phase !== "scanning" && s.importState?.phase !== "importing";

/** Whether an export may start: not over the import sheet, nor while one is already open. */
export const canStartExport = (s: Open) => !s.exportState && !s.importState;
