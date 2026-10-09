/**
 * Writes src/notices.json, the third-party notices on the About screen:
 * the bundled models, the Geist font, the interface's npm packages and the
 * Rust crates, each with its licence text.
 *
 *   bun run notices
 *
 * Needs cargo-about (`cargo install --locked cargo-about --features cli`). Run it after
 * changing Cargo.lock or package.json's dependencies; `bun test` says when
 * it's due. The file is checked in so building the app needs nothing extra.
 */
import { readdirSync, readFileSync, writeFileSync } from "node:fs";
import { join } from "node:path";
import { spawnSync } from "node:child_process";
import { fingerprint, lockedCrates, type NoticeGroup, type NoticeItem, type Notices } from "../src/notices";

const root = join(import.meta.dir, "..");
const models = join(root, "src-tauri", "models");

const texts: string[] = [];
const textIndex = new Map<string, number>();
/** Each text once; the same licence with a different copyright line is a different text. */
function text(body: string): number {
  const clean = body.replace(/\r\n/g, "\n").replace(/\s+$/, "") + "\n";
  let index = textIndex.get(clean);
  if (index === undefined) {
    index = texts.push(clean) - 1;
    textIndex.set(clean, index);
  }
  return index;
}
const modelText = (file: string) => text(readFileSync(join(models, file), "utf8"));

// ---- models: listed by hand, as models/README.md describes them ----

const modelGroup: NoticeGroup = {
  title: "Models",
  note: "Run on your computer to find subjects, skies and objects for masks.",
  items: [
    {
      name: "IS-Net (general use)",
      licence: "Apache-2.0, MIT",
      note:
        "Finds a photo's main subject. Xuebin Qin et al.'s weights, as exported to ONNX by rembg. " +
        "The authors' code is under the Apache License; they haven't given the weights a licence of their own.",
      texts: [modelText("U2NET-LICENSE"), modelText("REMBG-LICENSE")],
    },
    {
      name: "U²-Netp sky segmentation",
      licence: "Apache-2.0, MIT",
      note: "Finds the sky. U²-Netp, with xiongzhu666's sky weights.",
      texts: [modelText("U2NET-LICENSE"), modelText("SKYSEG-LICENSE")],
    },
    {
      name: "MobileNetV2dilated-C1 (ADE20K)",
      licence: "BSD-3-Clause",
      note: "Checks the sky model. From MIT CSAIL's semantic-segmentation-pytorch.",
      texts: [modelText("CSAIL-SEMSEG-LICENSE")],
    },
    {
      name: "EfficientSAM-Ti",
      licence: "Apache-2.0",
      note: "Finds the object inside a loop drawn on the photo. Yunyang Xiong et al., exported to ONNX by Kentaro Wada.",
      texts: [modelText("EFFICIENTSAM-LICENSE")],
    },
  ],
};

// ---- npm: what package.json's dependencies bring into the interface's bundle ----

interface PackageJson {
  name: string;
  version: string;
  license?: string;
  dependencies?: Record<string, string>;
}

const readPackage = (name: string): PackageJson =>
  JSON.parse(readFileSync(join(root, "node_modules", name, "package.json"), "utf8"));

/** The licence files a package ships, or, for one that ships none, another package of the same project's. */
const SAME_LICENCE_AS: Record<string, string> = {
  // Ships only a LICENSE.spdx; the Tauri project's texts are in @tauri-apps/api.
  "@tauri-apps/plugin-opener": "@tauri-apps/api",
};
function licenceFiles(name: string): number[] {
  const dir = join(root, "node_modules", SAME_LICENCE_AS[name] ?? name);
  const files = readdirSync(dir)
    .filter((file) => /^(licen[cs]e|copying|notice)/i.test(file) && !file.endsWith(".spdx"))
    .sort();
  if (files.length === 0) throw new Error(`${name} has no licence file; add it to SAME_LICENCE_AS`);
  return files.map((file) => {
    let body = readFileSync(join(dir, file), "utf8");
    // Vite's own licence goes on to list everything it bundles into itself,
    // none of which reaches the app; only its own notice applies.
    if (name === "vite") body = body.split(/^# Licenses of bundled dependencies/m)[0];
    return text(body);
  });
}

const rootPackage: PackageJson = JSON.parse(readFileSync(join(root, "package.json"), "utf8"));
const npmNames = new Set<string>();
const walk = (name: string) => {
  if (npmNames.has(name)) return;
  npmNames.add(name);
  for (const dep of Object.keys(readPackage(name).dependencies ?? {})) walk(dep);
};
for (const name of Object.keys(rootPackage.dependencies ?? {})) walk(name);

const FONT = "@fontsource-variable/geist";
npmNames.delete(FONT);
// Vite puts a few lines of its own into the bundle, to load its parts.
npmNames.add("vite");

const npmItem = (name: string, note?: string): NoticeItem => {
  const pkg = readPackage(name);
  return { name, version: pkg.version, licence: pkg.license ?? "see text", note, texts: licenceFiles(name) };
};

const fontGroup: NoticeGroup = {
  title: "Font",
  items: [{ ...npmItem(FONT, "Geist, by Vercel, packaged by Fontsource."), name: "Geist" }],
};
const npmGroup: NoticeGroup = {
  title: "Interface",
  items: [...npmNames].sort().map((name) => npmItem(name)),
};

// ---- Rust crates, from cargo-about ----

interface AboutOutput {
  licenses: { id: string; text: string; used_by: { crate: { name: string; version: string } }[] }[];
  crates: { package: { name: string; version: string }; license: string }[];
}

const about = spawnSync("cargo", ["about", "generate", "--format", "json", "--fail"], {
  cwd: join(root, "src-tauri"),
  encoding: "utf8",
  maxBuffer: 256 * 1024 * 1024,
  stdio: ["ignore", "pipe", "inherit"],
});
if (about.status !== 0) {
  console.error("cargo about failed. Install it with: cargo install --locked cargo-about --features cli");
  process.exit(1);
}
const crates: AboutOutput = JSON.parse(about.stdout);

const crateTexts = new Map<string, number[]>();
for (const licence of crates.licenses) {
  const index = text(licence.text);
  for (const { crate } of licence.used_by) {
    const key = `${crate.name} ${crate.version}`;
    crateTexts.set(key, [...(crateTexts.get(key) ?? []), index]);
  }
}
const crateGroup: NoticeGroup = {
  title: "Rust crates",
  items: crates.crates
    .filter(({ package: pkg }) => pkg.name !== "tonality")
    .map(({ package: pkg, license }) => ({
      name: pkg.name,
      version: pkg.version,
      licence: license,
      texts: [...new Set(crateTexts.get(`${pkg.name} ${pkg.version}`) ?? [])],
    }))
    .sort((a, b) => a.name.localeCompare(b.name) || a.version.localeCompare(b.version)),
};
const missing = crateGroup.items.filter((item) => item.texts.length === 0);
if (missing.length > 0) {
  console.error(`No licence text found for: ${missing.map((item) => item.name).join(", ")}`);
  process.exit(1);
}

const cargoLock = fingerprint(lockedCrates(readFileSync(join(root, "src-tauri", "Cargo.lock"), "utf8")));
const notices: Notices = { cargoLock, groups: [modelGroup, fontGroup, npmGroup, crateGroup], texts };
writeFileSync(join(root, "src", "notices.json"), JSON.stringify(notices) + "\n");
console.log(
  `Wrote src/notices.json: ${crateGroup.items.length} crates, ${npmGroup.items.length} npm packages, ` +
    `${modelGroup.items.length} models, ${texts.length} licence texts.`,
);
