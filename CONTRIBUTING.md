# Contributing to Tonality

Thanks for wanting to help. Tonality is trying to be a RAW editor with a lot of power behind an interface anyone can use, and getting there takes more than one pair of hands. Bug reports, testing on your camera's files, design ideas, docs and code are all welcome.

By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to help

- **Report a bug.** [Open an issue](https://github.com/cobysmckinney/tonality/issues/new/choose) with what you did, what you expected and what happened. Say which system and graphics card you're on. If a particular photo misbehaves, link the file (a cloud drive link is fine): RAW problems are almost impossible to fix without one.
- **Try your camera.** RAW formats vary a lot. Telling us "my Fujifilm X-T5 files open green" is genuinely useful.
- **Suggest something.** Open an issue describing the problem you want solved, not only the control you'd add. Tonality's aim is to stay simple, so the best ideas solve a real need without adding a panel.
- **Write code.** Pick an issue. [`good first issue`](https://github.com/cobysmckinney/tonality/labels/good%20first%20issue) marks the gentler ones, and `priority/P0` to `P3` show what matters most.

## Before you start on code

- **Say you're taking it.** Comment on the issue so two people don't do the same work. If there's no issue yet, open one first.
- **Agree on the approach for anything big.** A new tool, a change to the edit format or database, or a new dependency or model: sketch the plan in the issue before writing it, so a PR doesn't stall on a design question.
- **Keep the interface quiet.** The chrome is greyscale and the photo is the only colour. Labels are short, plain English. A new control has to earn its place: a good default beats a new slider.

## Setting up

You need [Rust](https://rustup.rs), [Bun](https://bun.sh), the [Tauri prerequisites](https://tauri.app/start/prerequisites/) for your system, and `libheif` (or build with `--no-default-features` to leave HEIC out).

```sh
git clone https://github.com/cobysmckinney/tonality.git
cd tonality
bun install
TONALITY_LIBRARY=/tmp/tonality-dev bun run tauri dev
```

Point `TONALITY_LIBRARY` at a scratch folder while developing so your real library (`~/Pictures/Tonality`) stays out of it. The first build takes a while; after that, interface changes reload live, and Rust changes rebuild and restart the app.

Two more switches help in debug builds:

- `TONALITY_FAKE_VOLUMES=/some/folder` treats any folder containing `DCIM` as a camera card, to test importing.
- `TONALITY_EVAL=/some/folder` runs each `.js` file dropped into that folder inside the window, then deletes it. It's a way to script clicks and drags when checking tools by hand.

## Tests

```sh
bun test                       # interface logic: crop, masks, presets (src/*.test.ts)
bunx tsc --noEmit              # type check
cd src-tauri && cargo test     # library, history, presets, export, masks and the GPU pipeline
```

The GPU tests skip themselves on a machine without a graphics adapter. Tests build their own synthetic files, so no sample photos are needed. A few slower checks run on real photos and are opt-in:

```sh
cd src-tauri
TONALITY_SAMPLES=/path/to/raws cargo test real_samples -- --ignored --nocapture
# Render real photos through the editor (optionally with a recipe) next to the camera's JPEG:
TONALITY_SAMPLES=/path/to/raws TONALITY_OUT=/tmp/out TONALITY_RECIPE='{"shadows":60}' \
  cargo test gpu_render -- --ignored --nocapture
# Render real photos with every built-in preset, to judge them by eye:
TONALITY_SAMPLES=/path/to/raws TONALITY_OUT=/tmp/out cargo test preset_sheet -- --ignored
```

## How it's put together

Tonality is a [Tauri 2](https://tauri.app) app: a Rust backend and a React + TypeScript interface.

**Backend** (`src-tauri/src`):

| File | What it does |
|---|---|
| `library.rs` | The library database (SQLite): photos, flags, albums, imports |
| `import.rs` | Scanning folders and cards, copying photos in |
| `media.rs` | Decoding files and reading their metadata |
| `thumbs.rs` | Thumbnails and previews |
| `volumes.rs` | Spotting camera cards |
| `develop.rs` | RAW to linear light, through [rawler](https://github.com/dnglab/dnglab) |
| `geometry.rs` | Crop, rotation and flip arithmetic |
| `gpu.rs`, `shaders/develop.wgsl` | The GPU pipeline: every adjustment is one pass of one shader, drawn at the size and crop on screen |
| `edit.rs` | The edit recipe |
| `history.rs` | Steps and branches, like a small git repository per photo |
| `presets.rs` | Built-in presets and the rules for applying them |
| `masks.rs` | Packing masks for the shader, brush coverage |
| `segment.rs` | Finding the subject, sky and objects with small ONNX models, run on the CPU by [tract](https://github.com/sonos/tract) |
| `export.rs` | Writing files out |
| `commands.rs` | The API the interface calls |

Integration tests live in `src-tauri/tests`, one file per area.

**Interface** (`src`): all state is in `store.ts` (zustand), `api.ts` wraps the backend commands, and the editor is under `components/editor`. Logic that can be tested on its own lives in plain modules (`crop.ts`, `masks.ts`, `presets.ts`) with `*.test.ts` next to them. `store.ts` imports Tauri and can't be loaded by `bun test`, so move logic out of it when you want to test it.

**Models** (`src-tauri/models`): the subject, sky and object models, about 145 MB together. [`models/README.md`](src-tauri/models/README.md) says where each came from, its licence, and how it was converted.

A few rules that aren't obvious from the code:

- **Originals are never written to.** Edits are recipes in the database; exports are new files.
- **Every edit is a history step.** Something new has to undo, redo, branch and paste like everything else, with a readable step name.
- **Thumbnails use the same shader as the editor**, so the grid matches what you see.
- **Mask parts are stored relative to the original file**, so they follow crops, turns and flips.
- **Changing a model**, or how its answer is refined, means changing its tag in `segment.rs` so cached mattes are worked out again.

## Pull requests

- Branch from `develop` and open the PR against it, one change per PR. Small PRs get reviewed faster. `develop` has the latest work and isn't always stable; `main` is stable, and releases are made from it.
- **Title**: one sentence describing the result for the user, in sentence case: "Zooming no longer stretches the photo while the next frame renders". No `feat:` or `fix:` prefixes.
- **Description**: what changed and why, how you checked it, and `Closes #123` for the issues it fixes. Add a screenshot or short clip for anything visible.
- **Checks**: `bun test`, `bunx tsc --noEmit` and `cargo test` pass. Changes to the shader or GPU pipeline come with a GPU test.
- **Docs**: if users would notice the change, update the [user guide](docs/guide.md) in the same PR.
- **Style**: match the code around you. Neither side has a formatter set up yet, so don't reformat files you aren't otherwise changing.
- **Dependencies** must be compatible with GPL-3.0-or-later. Don't commit sample photos or other large files.

PRs are squash-merged, so tidy commit history inside a PR doesn't matter. The PR title becomes the commit message.

Using AI tools to help write a contribution is fine. Every PR is judged on the same terms: does it work, is it tested, and is it the right change.

## Licence

Tonality is licensed under the [GNU General Public License v3.0 or later](LICENSE). By contributing, you agree that your contribution is licensed the same way. There is no CLA.
