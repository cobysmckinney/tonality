/**
 * The keyboard shortcuts, as the list the app shows when you press ? (and
 * the guide's table). Keep it in step with the key handlers in PhotoGrid.tsx
 * and Editor.tsx.
 */

export interface Shortcut {
  /** Each way to do it, as keys joined by +; "Left / Right" style pairs are one entry. */
  keys: string[];
  does: string;
}

export interface ShortcutGroup {
  title: string;
  shortcuts: Shortcut[];
}

export const SHORTCUTS: ShortcutGroup[] = [
  {
    title: "Grid and editor",
    shortcuts: [
      { keys: ["F"], does: "Favorite" },
      { keys: ["P"], does: "Pick" },
      { keys: ["X"], does: "Reject" },
      { keys: ["U"], does: "Unflag" },
      { keys: ["Delete"], does: "Delete" },
      { keys: ["Ctrl+C"], does: "Copy edits" },
      { keys: ["Ctrl+V"], does: "Paste edits" },
      { keys: ["Ctrl+E"], does: "Export" },
      { keys: ["?"], does: "Show this list" },
    ],
  },
  {
    title: "Grid",
    shortcuts: [
      { keys: ["Arrows"], does: "Move" },
      { keys: ["Shift+Arrows"], does: "Extend the selection" },
      { keys: ["Ctrl+A"], does: "Select all" },
      { keys: ["Enter"], does: "Open the photo" },
      { keys: ["Esc"], does: "Clear the selection" },
    ],
  },
  {
    title: "Editor",
    shortcuts: [
      { keys: ["Left", "Right"], does: "Previous or next photo" },
      { keys: ["Esc"], does: "Back, one step at a time" },
      { keys: ["Ctrl+Z"], does: "Undo" },
      { keys: ["Ctrl+Shift+Z", "Ctrl+Y"], does: "Redo" },
      { keys: ["\\"], does: "Hold to see the original" },
      { keys: ["Z"], does: "Fit or 100%" },
      { keys: ["J"], does: "Show clipped areas" },
    ],
  },
  {
    title: "Tools",
    shortcuts: [
      { keys: ["A"], does: "Adjust" },
      { keys: ["C"], does: "Crop" },
      { keys: ["M"], does: "Masks" },
      { keys: ["Shift+H"], does: "Heal" },
      { keys: ["Shift+P"], does: "Presets" },
      { keys: ["H"], does: "History" },
      { keys: ["I"], does: "Info" },
    ],
  },
  {
    title: "Crop, masks and heal",
    shortcuts: [
      { keys: ["Enter"], does: "Finish cropping" },
      { keys: ["O"], does: "Mask overlay" },
      { keys: ["[", "]"], does: "Smaller or larger brush or spot" },
      { keys: ["Alt+Drag"], does: "Erase with the brush" },
      { keys: ["Delete"], does: "Delete the chosen spot" },
      { keys: ["Alt+Click"], does: "Leave a speck marked as dust" },
    ],
  },
];

interface Open {
  menu: unknown;
  confirmRequest: unknown;
  importState: unknown;
  exportState: unknown;
  filmSheet: unknown;
}

/**
 * Whether a key press opens the list: `?`, outside a text field, with nothing
 * else (a menu, a question, a sheet) on top.
 */
export const opensShortcuts = (key: string, typing: boolean, s: Open) =>
  key === "?" && !typing && !s.menu && !s.confirmRequest && !s.importState && !s.exportState && !s.filmSheet;
