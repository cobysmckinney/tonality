#!/usr/bin/env node
// The app's version, kept in step across every file that states it.
//
//   node scripts/version.mjs                 print the current version
//   node scripts/version.mjs patch|minor|major
//   node scripts/version.mjs 1.2.3           set it (and print it)
//   node scripts/version.mjs --nightly       print the next nightly's version (changes nothing)
//
// tauri.conf.json is the source of truth; the Cargo workspace, package.json and both lock
// files follow it. Nightlies are `X.Y.(Z+1)-nightly.YYYYMMDD.N` after stable X.Y.Z, so they
// sort above the release they follow and below the next one (N from `NIGHTLY_BUILD`, else 0).
import { readFileSync, writeFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const path = (p) => join(root, p);
const read = (p) => readFileSync(path(p), "utf8");
const write = (p, s) => writeFileSync(path(p), s);

const SEMVER = /^(\d+)\.(\d+)\.(\d+)$/;
const CRATES = ["tonality", "tonality-core", "tonality-ml", "tonality-raw", "tonality-render"];

const current = JSON.parse(read("src-tauri/tauri.conf.json")).version;
const m = SEMVER.exec(current);
if (!m) fail(`tauri.conf.json has version ${current}, not X.Y.Z`);
const [major, minor, patch] = m.slice(1).map(Number);

const arg = process.argv[2];
if (!arg) {
  console.log(current);
} else if (arg === "--nightly") {
  const date = new Date().toISOString().slice(0, 10).replaceAll("-", "");
  const build = Number(process.env.NIGHTLY_BUILD ?? 0);
  console.log(`${major}.${minor}.${patch + 1}-nightly.${date}.${build}`);
} else {
  const next =
    arg === "major"
      ? `${major + 1}.0.0`
      : arg === "minor"
        ? `${major}.${minor + 1}.0`
        : arg === "patch"
          ? `${major}.${minor}.${patch + 1}`
          : arg.replace(/^v/, "");
  if (!SEMVER.test(next)) fail(`not a version: ${arg} (use patch, minor, major or X.Y.Z)`);
  if (compare(next, current) <= 0) fail(`${next} isn't newer than ${current}`);
  set(next);
  console.log(next);
}

function set(v) {
  replace("src-tauri/tauri.conf.json", /("version":\s*")[^"]+(")/, `$1${v}$2`);
  replace("package.json", /("version":\s*")[^"]+(")/, `$1${v}$2`);
  // The lock's own version, and the root package's.
  replace("package-lock.json", /^(\s{2}"version":\s*")[^"]+(")/m, `$1${v}$2`);
  replace(
    "package-lock.json",
    /("packages":\s*\{\s*"":\s*\{\s*"name":\s*"tonality",\s*"version":\s*")[^"]+(")/,
    `$1${v}$2`,
  );
  replace("Cargo.toml", /(\[workspace\.package\][^[]*?\nversion\s*=\s*")[^"]+(")/, `$1${v}$2`);
  for (const name of CRATES) {
    const at = new RegExp(`(\\[\\[package\\]\\]\\r?\\nname = "${name}"\\r?\\nversion = ")[^"]+(")`);
    replace("Cargo.lock", at, `$1${v}$2`);
  }
}

function replace(file, pattern, to) {
  const before = read(file);
  const after = before.replace(pattern, to);
  if (after === before && !pattern.test(before)) fail(`no version found in ${file}`);
  write(file, after);
}

function compare(a, b) {
  const pa = a.split(".").map(Number);
  const pb = b.split(".").map(Number);
  for (let i = 0; i < 3; i++) if (pa[i] !== pb[i]) return pa[i] - pb[i];
  return 0;
}

function fail(msg) {
  console.error(`version: ${msg}`);
  process.exit(1);
}
