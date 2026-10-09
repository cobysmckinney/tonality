/**
 * When the editor's keys are its own. A menu, a question, a sheet or the
 * list of shortcuts or the About screen open on top takes the keys; so does having no photo open.
 */

interface Open {
  confirmRequest: unknown;
  menu: unknown;
  shortcutsOpen: boolean;
  aboutOpen: boolean;
  importState: unknown;
  exportState: unknown;
  filmSheet: unknown;
  openId: number | null;
}

export const editorKeysBlocked = (s: Open) =>
  Boolean(s.confirmRequest || s.menu || s.shortcutsOpen || s.aboutOpen || s.importState || s.exportState || s.filmSheet) || s.openId === null;
