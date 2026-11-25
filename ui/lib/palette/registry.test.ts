import { describe, expect, it } from "vitest";
import {
  available,
  keyLabels,
  remember,
  search,
  shortcutGroups,
  shortcutText,
  type Command,
} from "./registry";

const run = () => {};
let photoOpen = false;
const commands: Command[] = [
  { id: "import", title: "Import photos…", group: "Library", keys: ["Mod+O"], run },
  { id: "undo", title: "Undo", group: "History", keys: ["Mod+Z"], when: () => photoOpen, run },
  {
    id: "redo",
    title: "Redo",
    group: "History",
    keys: ["Mod+Shift+Z", "Mod+Y"],
    when: () => photoOpen,
    run,
  },
  { id: "settings", title: "Settings…", group: "App", keywords: "preferences", run },
  { id: "before", title: "Show before (hold)", group: "View", keys: ["\\"] },
  { id: "export", title: "Export…", group: "Library", keys: ["Mod+Shift+E"], run },
];
const ids = (cs: { command: Command }[]) => cs.map((c) => c.command.id);

describe("available", () => {
  it("hides what can't run now, and shortcut-only entries", () => {
    photoOpen = false;
    expect(available(commands).map((c) => c.id)).toEqual(["import", "settings", "export"]);
    photoOpen = true;
    expect(available(commands).map((c) => c.id)).toEqual([
      "import",
      "undo",
      "redo",
      "settings",
      "export",
    ]);
  });
});

describe("search", () => {
  it("keeps registry order for an empty query, recently used first", () => {
    expect(ids(search(commands, "", []))).toEqual(ids(commands.map((command) => ({ command }))));
    expect(ids(search(commands, "  ", ["export", "redo"])).slice(0, 3)).toEqual([
      "export",
      "redo",
      "import",
    ]);
  });

  it("ranks by the title", () => {
    expect(ids(search(commands, "re", []))[0]).toBe("redo");
    expect(ids(search(commands, "exp", []))).toEqual(["export"]);
  });

  it("finds by group and keywords too", () => {
    expect(ids(search(commands, "history", []))).toEqual(["undo", "redo"]);
    expect(ids(search(commands, "prefs", []))).toEqual(["settings"]);
  });

  it("needs every word to match something", () => {
    expect(ids(search(commands, "history red", []))).toEqual(["redo"]);
    expect(ids(search(commands, "history import", []))).toEqual([]);
  });

  it("highlights only title matches", () => {
    const [r] = search(commands, "history u", []);
    expect(r.command.id).toBe("undo");
    expect(r.hits).toEqual([0]);
  });

  it("lifts a recently used command over a near-tie, not over a better match", () => {
    const two: Command[] = [
      { id: "a", title: "Zoom in", group: "View", run },
      { id: "b", title: "Zoom out", group: "View", run },
    ];
    expect(ids(search(two, "zoom", []))).toEqual(["a", "b"]);
    expect(ids(search(two, "zoom", ["b"]))).toEqual(["b", "a"]);
    expect(ids(search(two, "zoom in", ["b"]))).toEqual(["a"]);
  });
});

describe("remember", () => {
  it("moves the id to the front, once, keeping at most `max`", () => {
    expect(remember(["a", "b", "c"], "b")).toEqual(["b", "a", "c"]);
    expect(remember(["a", "b"], "c", 2)).toEqual(["c", "a"]);
  });
});

describe("shortcutGroups", () => {
  it("lists every command with keys by group, available or not", () => {
    photoOpen = false;
    expect(shortcutGroups(commands).map((g) => [g.group, g.items.map((c) => c.id)])).toEqual([
      ["Library", ["import", "export"]],
      ["History", ["undo", "redo"]],
      ["View", ["before"]],
    ]);
  });
});

describe("key labels", () => {
  it("names keys for Windows and Linux", () => {
    expect(keyLabels("Mod+Shift+E", false)).toEqual(["Ctrl", "Shift", "E"]);
    expect(shortcutText("Mod+Shift+E", false)).toBe("Ctrl+Shift+E");
    expect(keyLabels("Left", false)).toEqual(["←"]);
  });

  it("uses macOS symbols, modifiers in ⌥⇧⌘ order", () => {
    expect(keyLabels("Mod+Shift+E", true)).toEqual(["⇧", "⌘", "E"]);
    expect(shortcutText("Mod+K", true)).toBe("⌘K");
  });

  it("reads a plus key", () => {
    expect(keyLabels("Mod++", false)).toEqual(["Ctrl", "+"]);
    expect(keyLabels("+", false)).toEqual(["+"]);
  });
});
