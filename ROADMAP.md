# tonality roadmap

The working list of features and fixes. Work through **Now** from the top, one item at a time.
When **Now** is empty, take the next item from the earliest unfinished milestone under
**To 1.0**. Add new ideas to **Later**, and move finished items to **Done** with the date and
commit.

**How to work an item**
1. Branch from `main` (`git checkout -b <item-slug>`).
2. Settle the item's open questions with Coby before writing code.
3. Build it, add tests where logic is testable, and run `cargo test --workspace`,
   `cargo clippy --workspace --all-targets`, `cargo fmt --all --check`, `npm test`,
   `npm run check` and `npm run format:check` (`cargo fmt --all` and `npm run format` fix the
   last two).
4. Try it in `npm run tauri dev`. UI work isn't done until it has been seen in the app.
5. Merge into `main` and move the item to **Done**.

---

## Now

In review: fixes from a code audit of `main` at `65cce25` (2026-11-25), one pull request per
area. Move each to **Done** when it merges.

- [ ] CI and build tooling: time limits, `--locked`, a Tauri version check, LF checkouts, a
      nightly that can be retried (#164).
- [ ] Log file and updater (#165).
- [ ] Import: hash on copy, the Recycle Bin, leftover temporary files, queued imports (#166).
- [ ] Catalog and sidecars; catalog v8; a guard against updating to a build that can't read
      the library (#167). Also 0.2's "say so when the library is already open".
- [ ] Export: sizes, crop sharpness, lens corners, highlights, EXIF, failing fast (#168).
- [ ] Lens profiles and raw decoding (#169).
- [ ] AI masks: small raws, download prompts, version previews, stale downloads (#170).
- [ ] Edit: keyboard, crop and tool fixes (#171).
- [ ] Photo switching, thumbnails and Library state (#172).
- [ ] Tests: export rendering, raw decoding, the preview's GLSL (#173).
- [ ] Accessibility: the curve, crop and masks from the keyboard; contrast; screen readers
      (#174). Part of 0.9's accessibility item.
- [ ] Notices, README, the shortcut sheet and this roadmap (#175).

---

## To 1.0

tonality is free and open source. **1.0 means:** a photographer can import a shoot, cull, edit
and export it without opening another app; nothing they do can lose work; and the sidecar and
catalog formats are frozen, with a promise that later versions read them.

Each milestone is a release. The order inside one is a suggestion.

### 0.2 Trust

What's left after #58–#62.

- [ ] Say so when the library is already open in another window. Today the app exits with
      nothing on screen (the reason is in the log).
- [ ] Notice photos whose original has gone missing: mark them in the grid and offer to remove
      them, instead of failing with a decode error when opened.
- [ ] Never reuse a photo's id. `images.id` isn't `AUTOINCREMENT`, so removing the newest
      photos frees their ids for the next import; #59 clears the caches kept by id, and a
      catalog migration would end the hazard. Changes the catalog: do it before the format
      freezes.
- [ ] `export::tests::writes_ultra_hdr` is flaky: it compares two ICC profiles that record
      their creation time to the second, so it fails when they're made a second apart.
- [ ] Answer a thumbnail request whose job panicked, so the grid cell doesn't wait for ever.
- [ ] Sign the Windows installer. Open question: the route (SignPath Foundation, Azure
      Artifact Signing, or stay unsigned); see "Signing" in the README.
- [ ] Decide whether **GPU tests** becomes a required check on `main`.
- [ ] Export where there's no usable GPU. wgpu falls back to a software Vulkan driver when
      there is one (untested here), but Linux with only Mesa's OpenGL renderer has no float
      render target and the export fails. Open question: require software Vulkan and say so,
      or add a path that works without it.
- [ ] Stop an export in the middle of a file, not only between files (`Gpu::render` would
      check between tiles).
- [ ] Delete rejected photos in one go.
- [ ] `.x3f` (Sigma) back once rawler's decoder is finished.

### 0.3 A whole shoot

Today nothing works across photos: settings copy only between versions of one photo. This is
what most stands between tonality and someone using it for a real shoot, so it runs alongside
what's left of 0.2 rather than after it.

- [ ] Type a value into a slider.
- [ ] Copy and paste settings between photos, and sync them to a selection, choosing which
      panels. Build it as "apply this step, or this version, to other photos" (a cherry-pick):
      the Copy Settings dialog already applies chosen panels from a version as one step, so
      the source becomes another photo's step or version, and a preset is a saved one.
- [ ] Presets: save, apply, and apply on import.
- [ ] Auto (Light and white balance) for a selection.
- [ ] Culling: full screen, embedded previews, pick and reject at speed, compare several
      photos (from 0.5: culling comes before editing).
- [ ] Filters for camera, lens, ISO and date range (from 0.5: the metadata is stored already).
- [ ] Clipping shown on the photo, not only as the histogram's corner marks.
- [ ] A true before and after: "before" currently also undoes the crop. Add a split view.
- [ ] Full screen, and hide the right-hand panel.
- [ ] Tidy the versions that undo-then-edit creates (`main-2`, `main-3`…). They pile up and
      can only be deleted by hand. Open question: prune them, fold them away, or ask.
- [ ] Lens profile corrections on by default. Open question: this changes how photos already
      imported and never edited look, and what their unedited state hashes to.
- [ ] G and E work with Caps Lock on; the sort tooltip is right when sorting by name.

### 0.4 Raw basics

What a raw editor is expected to have.

- [ ] Highlight reconstruction. Blown highlights currently clip to neutral after white balance.
- [ ] Spot healing and clone.
- [ ] Perspective: keystone sliders, then guided or automatic upright.
- [x] Colour grading: shadows, midtones and highlights (#64).
- [ ] Black and white with a mixer.
- [ ] Hot and dead pixel removal.
- [ ] Lens corrections the camera wrote into the file for Fujifilm, OM/Olympus and Panasonic
      (only Sony and DNG are read today); the DNG optical centre; defringe.
- [ ] Masks: add, subtract and intersect shapes within one mask; colour range; duplicate and
      reorder; copy between photos.
- [ ] Brush and AI masks sharper than 1024 px on the long edge.
- [ ] Tone curve: a luminance mode (the master curve is per channel today).
- [ ] Crop: custom ratios, vertical flip, auto-level, guides that stay visible.
- [ ] Named snapshots of a step, and a list of what changed between any two steps.
- [ ] Better demosaicing as an option (PPG and one-pass Markesteijn today); monochrome
      sensors.

### 0.5 Library

- [ ] Keywords, colour labels, captions.
- [ ] Search (camera, lens, ISO and date filters moved to 0.3).
- [ ] Saved searches as smart albums.
- [ ] Open and switch libraries from the app (`TONALITY_LIBRARY` only, today).
- [ ] Import JPEG, TIFF and HEIC. Open question: how far the raw pipeline's assumptions go for
      photos that are already rendered.
- [ ] Group raw + JPEG pairs and bursts.
- [ ] A full metadata panel.
- [ ] Hold up with 50,000 photos: page `library_images`, don't lay out every row on every
      change, and don't walk every original and reconcile every sidecar on each launch.
- [ ] Limits and eviction for the thumbnail and mask caches.
- [ ] Back up the catalog on a schedule; verify originals against their hashes.
- [ ] Import: detect cards, rename on import.
- [ ] Background work at lower priority on macOS and Windows too (Linux only today).

### 0.6 Export

- [ ] Any long edge, megapixels or percentage; never only four fixed sizes.
- [ ] File name templates and numbering.
- [ ] Saved export recipes.
- [ ] Output sharpening.
- [ ] 16-bit PNG; AVIF and JPEG XL.
- [ ] Metadata choices: all, copyright only, none; add copyright and caption.
- [ ] Watermark.
- [ ] Choose which versions to export, not only "current" or "all"; stop tying the plain file
      name to a branch called exactly `main`.
- [ ] Ask before downloading AI models during an export.
- [ ] `tonality export` from the command line (the export renderer is already headless).
- [ ] Open in another editor.

### 0.9 Beta

Ship quality. No new features after this.

- [ ] Freeze the sidecar and catalog formats, with migration tests from every released
      version.
- [ ] Golden-image tests on a corpus of raw files from many cameras, in CI.
- [ ] Tests on macOS and Windows in CI (only Linux runs them today); component tests for the
      interface.
- [ ] `cargo-deny` or `cargo-audit` in CI.
- [ ] macOS Intel, `.deb` and Flatpak builds; an AppImage that runs on older glibc if that
      can be had.
- [ ] A user manual and a changelog.
- [ ] Memory: a guard and a plan for 60–100 megapixel files (the full decode is held as
      32-bit float before it's halved).
- [ ] Say in the app that sharpening, noise reduction and Texture only show fully at 1:1.
- [ ] Accessibility: focus order, focus traps in dialogs, the grid reachable by keyboard.
- [ ] Decide whether SegFormer-B0's non-commercial licence (below) is acceptable in a free
      app that working photographers will use for paid jobs; swap it if not.

### 1.0

- [ ] A release with no new features: only fixes found in the beta.

---

## Before a commercial release
- [ ] Replace SegFormer-B0 (sky/people masks): its license is non-commercial only.

## Later
_(new ideas go here)_
- P3 / HDR preview, once the webview can show one and there's a wide-gamut HDR display to check on (`docs/agx.md`, Later).
- Sync a library between machines: sidecars already merge like git, so a library in a synced
  folder could merge edits made on a laptop and a desktop.
- On-device AI denoise.
- Read Lightroom and darktable XMP, for people moving over.
- Panorama and HDR merge.
- Import in place, without copying into the library.
- Camera profiles (DCP), dual-illuminant matrices, `.cube` LUTs.
- Localisation; a light theme.
- Tethered shooting; printing.

## Done
- 2026-11-25 — Rate, pick and reject in Edit; an edited mark on grid and filmstrip thumbnails (#75).
- 2026-11-25 — Compare with another photo, side by side, to match a look (#71).
- 2026-11-25 — Merge from another version, panel by panel, with a live preview (#74).
- 2026-11-25 — History: what a step changed, on hover; hover a version to preview it; times by session instead of "now" on every step; Reset all history moved into the version menu; a resizable panel (#73).
- 2026-11-25 — Right panel: a tool strip under the histogram, the shooting info, Auto as a real button, sections that remember being open, and an option to keep one open at a time (#69).
- 2026-11-25 — A pixel readout and a navigator when zoomed in (#72).
- 2026-11-25 — White balance as a dropdown with Temperature and Tint always shown, hover to preview a preset, and an eyedropper (W) (#68).
- 2026-11-25 — Canvas: file name and camera moved to the filmstrip, a zoom menu, no "← →" hint (#70).
- 2026-11-25 — Command palette (Ctrl+K), with every shortcut listed under `?`: the keyboard shortcut reference (#67).
- 2026-11-25 — Errors and notices as toasts, where they happen (#66).
- 2026-11-23 — macOS build signed and notarised once the Apple secrets exist (#62).
- 2026-11-23 — GPU tests run in CI, on a macOS runner (#61).
- 2026-11-23 — Stop a running import or export (#60).
- 2026-11-23 — Remove photos from the library, to the system's Trash; read-only files import (#59).
- 2026-11-23 — A bad raw file can't take the app down; log file with panics in it; `.x3f` dropped (#58).
- 2026-11-23 — Colour grading (#64): one wheel with a dot each for shadows, midtones and highlights, plus Blending and Balance; the dots can be linked in a shape (analogous, complementary, triadic, split-complementary) so dragging one turns and grows all three (`feat/colour-grading`; `docs/colour-grading.md`).
- 2026-11-17 — Auto: Light (exposure, then range sliders, from a model of tone + AgX checked against the shader) and white balance (grey pixels from As Shot), in Edit, one undo step each (`auto-settings`; `docs/auto.md`).
- 2026-11-17 — Camera metadata in exports: EXIF (camera, lens, exposure, capture time, artist, copyright), star rating as EXIF + XMP, GPS behind an Export toggle (off by default); every format incl. Ultra HDR (`export-metadata`; `docs/export-metadata.md`).
- 2026-11-17 — Ultra HDR files without JFIF (MPF must follow SOI), one ICC profile per space (fixed a flaky test); repo formatted with rustfmt and Prettier, format checks added to the checklist (`leftovers-2`).
- 2026-11-17 — Display P3 and Ultra HDR export: AgX Base for P3 and AgX HDR (1000 nits) from Eary's scripts, each matching Blender's LUT at every grid point; export colour space (sRGB / Display P3, ICC embedded) and Ultra HDR JPEG (SDR base + gain map to AgX HDR) (`p3-hdr`; `docs/agx.md`).
- 2026-11-17 — Out-of-gamut colours carried to AgX: no Rec.2020 clamp after the camera matrix or in Colour; Tone's luminance floored so negative-luminance colours can't blow up; Dehaze keeps negatives; GPU tests for both (`out-of-gamut`).
- 2026-11-17 — Leftovers: values below black kept through noise reduction, own pool for prefetch decodes, catalog v4 (HEAD → branch key), in-app checks (`leftovers`, `b096c0e`).
- 2026-11-17 — Texture, Clarity, Dehaze in Effects and on masks (`presence`, `9b08681`; `docs/presence.md`).
- 2026-11-17 — AgX done properly: Blender's AgX Base from its parameters, checked against Blender's LUTs (`agx`, `8237c3b`; `docs/agx.md`).
- 2026-11-17 — Reset history, from Edit or for selected photos in the Library (`history-reset`). Cleaned the real library of 9 duplicate branches left by the old sidecar bug (backup in `~/Pictures/tonality-backup-2026-11-17-before-dedupe`), and fixed the merge so a copy in today's format can't capture the original.
- 2026-11-17 — Logo: italic Playfair `t` mark and wordmark, app icons, startup animation (`logo`, `e22ba27`).
- 2026-11-17 — Crop: aspect presets as icon buttons, orientation switch, 3:4; sidecar merge no longer duplicates histories when a new setting is added (`crop-buttons`, `7fbec47`).
- 2026-11-17 — Library: justified rows, photos edge to edge in their own shape; catalog v3 records each photo's shape (`library-cards`).
- 2026-11-17 — Masks rebuilt: floating list with black-and-white mask thumbnails (bottom right), settings in the side panel, red overlay on by default, hover to preview; local Hue, Vibrance, Color Tint, Sharpness and Noise Reduction (`masks-ui`).
- 2026-11-10 — Code review fixes (24 commits, `999a6a6`).
- 2026-11-10 — White balance presets: Daylight, Cloudy, Shade, Tungsten, Fluorescent, Flash (`5c95747`).
