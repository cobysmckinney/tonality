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

### 1. Reset a photo's history
**Status:** built on branch `history-reset`, seen in the app; waiting for Coby's review.
- [x] `Catalog::reset_history(images)`: deletes every branch, commit and unused edit state, and
  forgets the photo's shape; ratings, flags and albums stay. Test in `tonality-core`.
- [x] `history_reset` command: also removes the edited thumbnails and rewrites sidecars before
  returning (so a crash can't bring the old history back).
- [x] Edit: "Reset all history…" at the foot of the History panel. Library: "Reset History…"
  for the selected photos (Coby: both). Both confirm first; an open Edit session reloads.
- [ ] Coby tries it.

### 2. AgX done properly (the only tone mapping)
AgX is already the display transform (`agx()` in `develop.wgsl`), but it's an approximation:
- The sigmoid is a 6th-order polynomial fit (from the "minimal AgX" approximations), not
  AgX's actual curve.
- The inset/outset matrices are the commonly copied ones made for **sRGB** input, while
  tonality's working space is **Rec.2020**. Check whether that mismatch is real.
- The Rec.2020→sRGB matrix is rounded to 4 digits, and the `pow(2.2)` after the outset needs a
  reason (or removal).
- Out-of-gamut values are only clipped at the end.

The same shader drives the preview (WebGL via naga), export (wgpu), the mask luminance range
and the histogram, so a fix lands everywhere at once.

**Research first, from primary sources, and write down the math before changing the shader:**
- Troy Sobotka's original AgX (github.com/sobotka/AgX) and his writings on it.
- Blender 4.x's AgX by Eary Chow (github.com/EaryChow/AgX and `AgX_LUT_Gen`): the inset
  (primary attenuation and rotation in Rec.2020), the log2 encoding range, the tunable
  sigmoid, the outset (restoring purity), and the "Base" look.
- Blender's OCIO config (the actual transform chain shipped with Blender).
- Shader ports to compare against (three.js `AgXToneMapping`, Filament), noting where they
  approximate.

**Plan**
- [ ] Write the transform chain, with parameters and sources, in a short doc
  (`docs/agx.md`): input space, inset, log encoding range, sigmoid, outset, EOTF/display
  encoding, target display (sRGB; later P3).
- [ ] Generate reference values from the real thing (Blender's OCIO config through PyOpenColorIO,
  or the reference scripts) for a grid of test colors: greys, saturated primaries, very
  bright values, out-of-gamut values.
- [ ] Compute the matrices in Rust from their parameters (not pasted constants), pass them to
  the shader, and replace the polynomial with the real sigmoid (exact formula or a LUT
  generated from it).
- [ ] A Rust test comparing a CPU version of the transform to the references, within a
  stated tolerance; a GPU test that the shader matches the CPU version.
- [ ] Check mid grey, white point, black point and how saturated highlights desaturate, on the
  sample photos. Note in the commit that existing edits will look different.
- [ ] Remove any wording or code suggesting other tone mappers.

### 3. Texture, Clarity and Dehaze (global and on masks)
The most used local adjustments in other editors, and missing globally too. Each compares a
pixel with a blurred copy of its neighbourhood, so they need a multi-scale blur (or a
precomputed blurred pyramid) rather than a per-pixel formula.

**Plan**
- [ ] Research how they're usually built (local contrast at fine / medium scale; dehaze via
  a dark-channel or atmospheric-light estimate) and pick methods that run on the GPU in the
  preview and match in export, tiled.
- [ ] Global sliders (Effects or a new Presence section) and the same three on masks.
- [ ] Watch preview speed: blurs at full resolution while zoomed, and at tile edges in export.

### 4. Leftovers from the code review
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
- 2026-11-17 — Logo: italic Playfair `t` mark and wordmark, app icons, startup animation (`logo`, `e22ba27`).
- 2026-11-17 — Crop: aspect presets as icon buttons, orientation switch, 3:4; sidecar merge no longer duplicates histories when a new setting is added (`crop-buttons`, `7fbec47`).
- 2026-11-17 — Library: justified rows, photos edge to edge in their own shape; catalog v3 records each photo's shape (`library-cards`).
- 2026-11-17 — Masks rebuilt: floating list with black-and-white mask thumbnails (bottom right), settings in the side panel, red overlay on by default, hover to preview; local Hue, Vibrance, Color Tint, Sharpness and Noise Reduction (`masks-ui`).
- 2026-11-10 — Code review fixes (24 commits, `999a6a6`).
- 2026-11-10 — White balance presets: Daylight, Cloudy, Shade, Tungsten, Fluorescent, Flash (`5c95747`).
