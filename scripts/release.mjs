#!/usr/bin/env node
// Cut a stable release from what's on GitHub's main, without touching your checkout:
//
//   npm run release              next patch (0.1.0 → 0.1.1)
//   npm run release -- minor     0.1.0 → 0.2.0
//   npm run release -- major     0.1.0 → 1.0.0
//   npm run release -- 1.4.2     exactly that
//   npm run release -- nightly   build a nightly now instead of waiting for tonight's
//
// Starts .github/workflows/release.yml (or nightly.yml), which bumps the version on main,
// tags it, builds every platform and publishes. Needs the GitHub CLI (`gh`), signed in.
import { execFileSync } from "node:child_process";

const arg = process.argv[2] ?? "patch";
if (!/^(patch|minor|major|nightly|v?\d+\.\d+\.\d+)$/.test(arg)) {
  console.error(`release: expected patch, minor, major, nightly or X.Y.Z, not "${arg}"`);
  process.exit(1);
}
const gh = (...args) => execFileSync("gh", args, { encoding: "utf8" }).trim();

const repo = gh("repo", "view", "--json", "nameWithOwner", "--jq", ".nameWithOwner");
const workflow = arg === "nightly" ? "nightly.yml" : "release.yml";
const inputs = arg === "nightly" ? [] : ["-f", `version=${arg.replace(/^v/, "")}`];
gh("workflow", "run", workflow, "--repo", repo, "--ref", "main", ...inputs);

const what = arg === "nightly" ? "A nightly build" : `A ${arg} release`;
console.log(`${what} of ${repo}'s main has started.`);
console.log(`Follow it: https://github.com/${repo}/actions/workflows/${workflow}`);
console.log(`Releases:  https://github.com/${repo}/releases`);
