import { describe, expect, test } from "bun:test";
import { readdirSync, readFileSync } from "node:fs";
import { join } from "node:path";
import { fingerprint, lockedCrates, Notices } from "./notices";
import noticesJson from "./notices.json";

const root = join(import.meta.dir, "..");
const notices = noticesJson as Notices;
const group = (title: string) => notices.groups.find((g) => g.title === title)!;
const STALE = "the third-party notices are out of date: run `bun run notices`";

describe("the third-party notices", () => {
  test("were made from the crates Cargo.lock names now", () => {
    const lock = readFileSync(join(root, "src-tauri", "Cargo.lock"), "utf8");
    expect(notices.cargoLock, STALE).toBe(fingerprint(lockedCrates(lock)));
  });

  test("list every crate at a version Cargo.lock names", () => {
    const locked = new Set(lockedCrates(readFileSync(join(root, "src-tauri", "Cargo.lock"), "utf8")));
    const crates = group("Rust crates").items;
    expect(crates.length).toBeGreaterThan(100);
    for (const item of crates) expect(locked.has(`${item.name} ${item.version}`), item.name).toBe(true);
    // The decoder is LGPL and linked into the app; its notice must be there.
    expect(crates.some((item) => item.name === "rawler")).toBe(true);
  });

  test("list each package the interface depends on, at the installed version", () => {
    const pkg = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
    const listed = [...group("Interface").items, ...group("Font").items];
    for (const name of Object.keys(pkg.dependencies)) {
      if (name === "@fontsource-variable/geist") continue;
      const item = listed.find((i) => i.name === name);
      expect(item, `${name}: ${STALE}`).toBeDefined();
      const installed = JSON.parse(readFileSync(join(root, "node_modules", name, "package.json"), "utf8")).version;
      expect(item!.version, `${name}: ${STALE}`).toBe(installed);
    }
    expect(group("Font").items[0].licence).toBe("OFL-1.1");
  });

  test("give every model's licence file", () => {
    const models = join(root, "src-tauri", "models");
    const files = readdirSync(models).filter((file) => file.endsWith("-LICENSE"));
    const shown = new Set(group("Models").items.flatMap((item) => item.texts));
    for (const file of files) {
      const body = readFileSync(join(models, file), "utf8").replace(/\r\n/g, "\n").replace(/\s+$/, "") + "\n";
      expect(shown.has(notices.texts.indexOf(body)), file).toBe(true);
    }
    expect(group("Models").items.length).toBe(4);
  });

  test("give every item a licence text", () => {
    for (const g of notices.groups) {
      for (const item of g.items) {
        expect(item.texts.length, item.name).toBeGreaterThan(0);
        for (const index of item.texts) expect(notices.texts[index]?.length, item.name).toBeGreaterThan(0);
      }
    }
  });
});

describe("Cargo.lock's crates", () => {
  test("are read as name and version, without Tonality itself", () => {
    const lock = `version = 4

[[package]]
name = "adler2"
version = "2.0.1"
source = "registry+https://github.com/rust-lang/crates.io-index"

[[package]]
name = "tonality"
version = "0.1.0"
dependencies = [
 "adler2",
]

[[package]]
name = "abc"
version = "1.0.0"
`;
    expect(lockedCrates(lock)).toEqual(["abc 1.0.0", "adler2 2.0.1"]);
  });

  test("change the fingerprint when one changes", () => {
    expect(fingerprint(["a 1.0.0"])).not.toBe(fingerprint(["a 1.0.1"]));
    expect(fingerprint(["a 1.0.0"])).toBe(fingerprint(["a 1.0.0"]));
  });
});
