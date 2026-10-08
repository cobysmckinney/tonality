/**
 * When the editor's keys are its own. A menu, a question or a sheet open on
 * top takes the keys; so does having no photo open.
 */

interface Open {
  confirmRequest: unknown;
  menu: unknown;
  importState: unknown;
  exportState: unknown;
  openId: number | null;
}

export const editorKeysBlocked = (s: Open) =>
  Boolean(s.confirmRequest || s.menu || s.importState || s.exportState) || s.openId === null;
