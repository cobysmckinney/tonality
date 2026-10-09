/**
 * The third-party notices shown on the About screen: everything Tonality is
 * built on and ships, with the licence each is used under.
 *
 * `notices.json` is written by `bun run notices` (scripts/notices.ts), from
 * cargo-about for the Rust crates, node_modules for the interface's packages,
 * and the licence files beside the models. `notices.test.ts` fails when
 * Cargo.lock or package.json no longer match it.
 */

export interface NoticeItem {
  name: string;
  version?: string;
  /** The licence as an SPDX expression, e.g. "MIT OR Apache-2.0". */
  licence: string;
  /** What it is, where that isn't obvious from the name. */
  note?: string;
  /** Indexes into `Notices.texts`: the licence texts, with their copyright lines. */
  texts: number[];
}

export interface NoticeGroup {
  title: string;
  note?: string;
  items: NoticeItem[];
}

export interface Notices {
  /** `fingerprint(lockedCrates(Cargo.lock))` when these were written. */
  cargoLock: string;
  groups: NoticeGroup[];
  /** Each licence text once; items refer to them by index. */
  texts: string[];
}

/** The crates Cargo.lock names, as "name version", leaving out Tonality itself. */
export function lockedCrates(cargoLock: string): string[] {
  const crates: string[] = [];
  for (const block of cargoLock.split("[[package]]").slice(1)) {
    const name = /^name = "(.+)"$/m.exec(block)?.[1];
    const version = /^version = "(.+)"$/m.exec(block)?.[1];
    if (name && version && name !== "tonality") crates.push(`${name} ${version}`);
  }
  return crates.sort();
}

/**
 * A short fingerprint of Cargo.lock's crates, kept in notices.json to tell
 * when the notices were made from a different set. Not for security.
 */
export function fingerprint(lines: string[]): string {
  let hash = 0x811c9dc5;
  for (const char of lines.join("\n")) {
    hash ^= char.codePointAt(0)!;
    hash = Math.imul(hash, 0x01000193) >>> 0;
  }
  return hash.toString(16).padStart(8, "0");
}
