# Tonality

A RAW photo editor and library (Tauri 2: Rust backend in `src-tauri/`, React + TypeScript interface in `src/`, one WGSL develop shader). Licensed GPL-3.0-or-later.

## What it's for

A "one size fits all" editor: a lot of power behind an interface someone can use without a four-hour tutorial. Think Lightroom's workflow with darktable's openness, but calmer than either. When choosing between exposing a control and making a good default, prefer the default. When adding a feature, ask how someone who has never edited a RAW would find and understand it.

- **Look**: greyscale, quiet "clean room" chrome where the photo is the only colour on screen. White marks what is active or selected. Colour only where it means something (pick/reject, clipping, the mask overlay). No decorative accents, gradients or floating cards.
- **Words**: plain, short English in labels, tooltips, docs and commits. Say what happens, not how it's built. Match the voice of the existing text.
- **Originals are never touched.** Edits are recipes in the library database; exports are new files. Don't propose converting the library to DNG (decided against; DNG is fine as an export or opt-in import option).

## Work tracking

- GitHub issues on `cobysmckinney/tonality` are the only to-do list. There is no TODO.md. Labels: `bug`, `enhancement`, `priority/P0`–`P3` (exclusive), area labels (`area/…`), `good first issue`, `help wanted`.
- Something you notice but aren't fixing becomes an issue (search first to avoid duplicates), not a code comment or a note in a file.

## Workflow

- `develop` is the default branch and holds the latest work; it isn't guaranteed stable, and nightly builds will come from it. `main` is stable: full releases ship from it, and it only moves when `develop` is brought over for a release.
- Every change goes through a branch and a pull request into `develop`, squash-merged. Never commit to or push `develop` or `main` (GitHub rules block it anyway).
- A release is a pull request from `develop` into `main`, merged with a merge commit (not squashed) so the two branches share history. An urgent fix goes into `develop` like anything else and then out in a release; there are no hotfix branches off `main`.
- Branch from an up-to-date `develop`, named after the change (`fix-mask-part-target`, `crop-zoom`).
- Claude may commit, push the branch and open the PR once the change is tested. Merging is the maintainer's call; never merge, and never force-push a branch someone else is working on.
- The PR body says what changed for the user and why, how it was checked, and `Closes #N` for each issue it fixes.
- Commit and PR titles: one sentence-case line describing the outcome for the user ("Zooming no longer stretches the photo while the next frame renders"). No `feat:`/`fix:` prefixes. Body: a short paragraph or bullets on what changed and why.

## Commands

```sh
bun install
bun run tauri dev                     # the app, against ~/Pictures/Tonality unless TONALITY_LIBRARY is set
bun test                              # interface unit tests (src/*.test.ts)
bunx tsc --noEmit                     # type check (bun run build also runs it)
cd src-tauri && cargo test            # backend + GPU pipeline tests (GPU tests skip without an adapter)
cd src-tauri && cargo clippy --all-targets
```

Before opening a PR: `bun test`, `bunx tsc --noEmit` and `cargo test` pass, and no new clippy warnings. A change to the shader or GPU pipeline also gets a GPU test in `src-tauri/tests/` or `gpu.rs`.

## Checking a change in the running app

Interface changes are checked in the real app, not only by tests:

- Run against a scratch library, never the real one: `TONALITY_LIBRARY=<scratch dir> bun run tauri dev`.
- `TONALITY_FAKE_VOLUMES=<dir containing DCIM>` (debug builds) shows a folder as a camera card, to fill a scratch library by importing.
- `TONALITY_EVAL=<dir>` (debug builds) runs each `.js` file put in that folder inside the window, then deletes it. Use it to script clicks and drags.
- Editing `src-tauri/` makes `tauri dev` rebuild and restart (a few minutes). Frontend edits hot-reload, but window-level key handlers and the zustand store don't reload cleanly: reload the page or restart before judging a change.

## Layout

- `src-tauri/src`: `library.rs` database · `import.rs` scan and copy · `media.rs` decoding and metadata · `thumbs.rs` generated images · `volumes.rs` card detection · `develop.rs` RAW to linear light (rawler) · `geometry.rs` crop and rotation · `gpu.rs` the wgpu pipeline · `shaders/develop.wgsl` the single develop pass · `edit.rs` the edit recipe · `history.rs` steps and branches · `presets.rs` · `masks.rs` mask packing and brush coverage · `segment.rs` found masks (tract, CPU) · `export.rs` · `commands.rs` the API the interface calls.
- `src-tauri/tests`: integration tests per module, on synthetic files. Real-photo tests are `#[ignore]`d and read `TONALITY_SAMPLES`.
- `src`: React interface. All state is in `store.ts` (zustand); `api.ts` wraps the Tauri commands; the editor is under `components/editor`. Pure logic lives in plain modules (`crop.ts`, `masks.ts`, `presets.ts`) so `bun test` can cover it. `store.ts` imports Tauri and can't be loaded by `bun test`, so move logic you want to test out of it.
- `src-tauri/models`: the bundled ONNX models (about 145 MB), with their licences and how each was made in `models/README.md`.

## Rules that aren't obvious from the code

- Every edit is a history step. A new kind of change has to undo, redo, branch and paste like the others, and have a readable step name.
- Thumbnails are drawn by the same shader as the editor, so the grid matches. Anything that changes the look has to change both.
- Changing a model, or how its answer is refined, means changing its tag in `segment.rs` so cached mattes are found again.
- Mask parts are stored relative to the original file, so they follow crops, turns and flips. Keep it that way.
- Keep `docs/guide.md` (the user guide) in step with behaviour: a change users would notice updates it in the same PR. The README is the pitch; only change it for things worth advertising.
- Dependencies and models must be compatible with GPL-3.0-or-later. A new model needs its licence file and an entry in `models/README.md`. rawler is LGPL-2.1 and statically linked, which is why the source must stay public.
- Don't commit RAW sample files or other large binaries. Test fixtures are generated in code.
