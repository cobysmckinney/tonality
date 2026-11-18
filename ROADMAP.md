# tonality roadmap

The working list of features and fixes. Work through **Now** from the top, one item at a time.
Add new ideas to **Later**, and move finished items to **Done** with the date and commit.

**How to work an item**
1. Branch from `main` (`git checkout -b <item-slug>`).
2. Settle the item's open questions with Coby before writing code.
3. Build it, add tests where logic is testable, and run `cargo test --workspace`,
   `cargo clippy --workspace --all-targets`, `npm test` and `npm run check`.
4. Try it in `npm run tauri dev`. UI work isn't done until it has been seen in the app.
5. Merge into `main` and move the item to **Done**.

---

## Now

### 1. AgX done properly (the only tone mapping)
**Status:** built on branch `agx`, seen in the app; waiting for Coby's review. The math, its
sources and the checks are in `docs/agx.md`.
- [x] Researched Sobotka's AgX, Eary Chow's AgX_LUT_Gen and Blender's config; wrote the chain down.
- [x] References from Blender's real config and LUTs through OpenColorIO
  (`scripts/agx_reference.py` → `crates/tonality-core/tests/data/agx_reference.json`). The written
  math reproduces Blender's LUT grid points (mean 8e-6).
- [x] `tonality_core::agx`: matrices computed from AgX's parameters (they match Blender's to 8
  digits), CPU reference; the shader gets them in the params uniform. The real sigmoid, guard
  rails and 40% hue restore replace the polynomial.
- [x] Tests: matrices, CPU vs exact and vs Blender, GPU shader vs CPU (`tests/agx_gpu.rs`).
- [x] Compared old and new on real photos: grey 0.50 → 0.46, shadows more open, colour holds better.
- [ ] Coby tries it (existing edits will look a little different).
- Later: Display P3 output; keep negatives through the pipeline so AgX's first guard rail sees
  out-of-gamut colours (goes with the noise-reduction leftover).

### 2. Texture, Clarity and Dehaze (global and on masks)
The most used local adjustments in other editors, and missing globally too. Each compares a
pixel with a blurred copy of its neighbourhood, so they need a multi-scale blur (or a
precomputed blurred pyramid) rather than a per-pixel formula.

**Plan**
- [ ] Research how they're usually built (local contrast at fine / medium scale; dehaze via
  a dark-channel or atmospheric-light estimate) and pick methods that run on the GPU in the
  preview and match in export, tiled.
- [ ] Global sliders (Effects or a new Presence section) and the same three on masks.
- [ ] Watch preview speed: blurs at full resolution while zoomed, and at tile edges in export.

### 3. Leftovers from the code review
- [ ] Try in the app: the content security policy (watch the console for blocked loads), the
  white balance presets (does "Fluorescent" fit?), the library thumbnail after leaving Edit
  quickly, and switching photos quickly with masks.
- [ ] Shadows lifted/tinted before noise reduction (`crates/tonality-raw/src/preview.rs`):
  negatives are clipped to 0 before NR. Keep them through NR; clamp later. Compare before
  and after on real photos.
- [ ] Background prefetch can make a zoom-in wait right after opening a photo
  (`src-tauri/src/images.rs`: `detail_region` waits on the low-priority decode).
- [ ] Optional: a foreign key from branch heads to branches (`crates/tonality-core/src/catalog.rs`);
  needs a schema migration.

---

## Before a commercial release
- [ ] Replace SegFormer-B0 (sky/people masks): its license is non-commercial only.

## Later
_(new ideas go here)_

## Done
- 2026-11-17 — Reset history, from Edit or for selected photos in the Library (`history-reset`). Cleaned the real library of 9 duplicate branches left by the old sidecar bug (backup in `~/Pictures/tonality-backup-2026-11-17-before-dedupe`), and fixed the merge so a copy in today's format can't capture the original.
- 2026-11-17 — Logo: italic Playfair `t` mark and wordmark, app icons, startup animation (`logo`, `e22ba27`).
- 2026-11-17 — Crop: aspect presets as icon buttons, orientation switch, 3:4; sidecar merge no longer duplicates histories when a new setting is added (`crop-buttons`, `7fbec47`).
- 2026-11-17 — Library: justified rows, photos edge to edge in their own shape; catalog v3 records each photo's shape (`library-cards`).
- 2026-11-17 — Masks rebuilt: floating list with black-and-white mask thumbnails (bottom right), settings in the side panel, red overlay on by default, hover to preview; local Hue, Vibrance, Color Tint, Sharpness and Noise Reduction (`masks-ui`).
- 2026-11-10 — Code review fixes (24 commits, `999a6a6`).
- 2026-11-10 — White balance presets: Daylight, Cloudy, Shade, Tungsten, Fluorescent, Flash (`5c95747`).
