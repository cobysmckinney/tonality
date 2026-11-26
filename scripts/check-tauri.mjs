#!/usr/bin/env node
// Checks that each @tauri-apps npm package is on the same major.minor as its Rust crate.
//
//   node scripts/check-tauri.mjs
//
// `tauri build` refuses to run when they differ, and only the release and nightly builds run
// it. This is the same comparison, made from the two lock files, so a pull request that bumps
// one side without the other fails CI instead of the next nightly.
//
// @tauri-apps/api pairs with the `tauri` crate, @tauri-apps/plugin-x with `tauri-plugin-x`.
import { readFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const read = (p) => readFileSync(join(root, p), "utf8");

const npm = JSON.parse(read("package-lock.json")).packages;
const cargo = read("Cargo.lock");

const crateVersion = (name) =>
  new RegExp(`\\[\\[package\\]\\]\\r?\\nname = "${name}"\\r?\\nversion = "([^"]+)"`).exec(
    cargo,
  )?.[1];
const minor = (version) => version.split(".").slice(0, 2).join(".");

const mismatched = [];
let checked = 0;
for (const [path, pkg] of Object.entries(npm)) {
  const name = /^node_modules\/@tauri-apps\/(api|plugin-[a-z0-9-]+)$/.exec(path)?.[1];
  if (!name) continue;
  const crate = name === "api" ? "tauri" : `tauri-${name}`;
  const version = crateVersion(crate);
  if (!version) {
    mismatched.push(`@tauri-apps/${name} ${pkg.version} has no ${crate} crate in Cargo.lock`);
  } else if (minor(version) !== minor(pkg.version)) {
    mismatched.push(
      `${crate} ${version} (Cargo.lock) : @tauri-apps/${name} ${pkg.version} (package-lock.json)`,
    );
  }
  checked++;
}

if (checked === 0) {
  console.error("check-tauri: no @tauri-apps packages found in package-lock.json");
  process.exit(1);
}
if (mismatched.length > 0) {
  console.error(
    "check-tauri: the Tauri npm packages and Rust crates must be on the same major.minor, " +
      "or `tauri build` (every nightly and release) refuses to run:",
  );
  for (const line of mismatched) console.error(`  ${line}`);
  console.error("Bump the other side in the same pull request.");
  process.exit(1);
}
console.log(`check-tauri: ${checked} Tauri npm packages match their crates`);
