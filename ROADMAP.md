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

### 1. Rebuild the masks UI
The masks panel (`ui/lib/edit/MaskPanel.svelte`, 648 lines) feels cluttered. Make it more intuitive,
and show the mask area in red.

What's there today: Done button in a header; a row of Linear/Radial/Brush buttons; a "Detect" grid
(Subject, Background, Sky, People, Select Object); the mask list (rename, hide, delete); then the
selected mask's hint, shape settings, AI status, Invert, Amount, luminance range, the "Show overlay"
checkbox and the adjustment sliders.
The red overlay already exists, but it's off by default: `mask_overlay` in
`crates/tonality-render/shaders/develop.wgsl` (red at 55%), toggled by `session.maskOverlay` or `O`.

**Decided (2026-11-17):** the clutter is the settings once a mask exists, and it's hard to see
what a mask affects. Lightroom-style layout: the masks float over the photo as black-and-white
thumbnails; the selected mask's settings go in the side panel. Red overlay on by default.

**Status:** built on branch `masks-ui`, seen in the app; waiting for Coby's review before merging.
- [x] Floating mask list over the photo (`MaskList.svelte`): black-and-white thumbnail of each
  mask (rendered by the shader's new mask-only mode), "+" menu to create one, hide/delete,
  double-click to rename, collapsible.
- [x] Side panel (`MaskPanel.svelte`): the selected mask's name and kind, then collapsible
  sections: Gradient/Brush/Refine tools, Mask (Amount, Invert), Light, Color, Luminance Range
  (closed by default). With no masks yet, the ways to make one (`MaskCreate.svelte`).
- [x] Red overlay on by default, toggled with the checkbox or `O`.
- [x] Hovering a mask in the list shows its area in red.
- [x] List keyed by a stable per-mask id instead of position.
- [x] Masks past the 16 drawn are marked "Not drawn: over 16" in the list.
- [x] List at the bottom right of the photo (Coby, 2026-11-17); "+" swaps the list for the
  ways to make a mask.
- [x] More local adjustments: Hue, Vibrance (Color), Color Tint (hue + amount, keeps
  brightness), Sharpness (negative softens) and Noise Reduction (Detail). Decided against
  per-mask tone curve and colour mixer (heavy, rarely used) and frame-wide effects (vignette,
  grain, lens, crop).
- [ ] Coby tries it: every kind (linear, radial, brush, each AI preset, lasso object), rename,
  hide, delete, undo; the floating list in a small window.

### 2. Library cards: image fills the card, no border
`ui/lib/library/Grid.svelte`: the image sits inside `.frame` with an 8px inset
(`max-width: calc(100% - 16px)`), `object-fit: contain` and a grey background, so every
photo is letterboxed inside a border.

**Open questions**
- Fill by cropping each image to a square (`object-fit: cover`), or a justified layout
  (rows of varying widths, so each card has its photo's own shape and nothing is cropped)?
  The justified layout looks best, but it's more work: the grid's virtualized layout
  (`layout.cells`) has to calculate rows.

**Plan**
- [ ] Remove the inset and background; the image fills `.frame` to its edges with the card's
  rounded corners.
- [ ] New selected and focused styles (an outline drawn over the image) and hover.
- [ ] Place the rating, flag and edited badges over the image with a soft shadow so they
  stay readable.
- [ ] Rejected photos are still dimmed; check placeholders while thumbnails load.
- [ ] Check scroll performance with a large library.

### 3. Crop: aspect ratios as icon buttons
`ui/lib/edit/CropPanel.svelte`, `ASPECTS` in `ui/lib/edit/crop.ts` (Free, Original, 1:1, 4:5, 5:7,
2:3, 9:16).

**Plan**
- [ ] Each aspect becomes a button with an icon: a rectangle drawn in that ratio (a dashed one
  for Free, a photo shape for Original) with the label under it.
- [ ] Turn the portrait/landscape choice into one rotate button that flips the icons too.
- [ ] Consider adding 16:9 (landscape of 9:16 already covers it when flipped; check
  the labels read right in both orientations) and 3:4.
- [ ] Keep `session.cropAspect` and the highlighted button in sync (regression check).

### 4. Logo and brand
**Open questions**
- What should tonality look and feel like? Words, references, colors, and any wordmark ideas.

**Plan**
- [ ] Two or three directions for a mark plus the "tonality" wordmark, made as SVG; pick one.
- [ ] Generate the app icons from the SVG (`npx tauri icon`, replacing `src-tauri/icons/*`).
- [ ] Use it in the app: library header or title area, and the about/empty state.
- [ ] Optional: an accent color that goes with it, set in the design tokens.

### 5. Reset a photo's history
A single action that takes a photo back to as-shot and removes **all** its edit history and
branches. Can't be undone.

**Open questions**
- Just the open photo, or also the selected photos in the library?

**Plan**
- [ ] Rust: `history_reset(image)` in `src-tauri/src/history.rs` → `tonality-core` deletes every
  branch, commit and unused edit state for the image, makes a fresh `main` with the default
  edit, and rewrites the sidecar. Tests in `tonality-core`.
- [ ] Remove the cached edited thumbnail and AI mask rasters for the image.
- [ ] UI: "Reset all history…" in the history panel (and library context menu if chosen), with a
  confirmation dialog that names what will be lost.
- [ ] Make sure an open Edit session reloads cleanly afterwards.

### 6. AgX done properly (the only tone mapping)
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

### 7. Texture, Clarity and Dehaze (global and on masks)
The most used local adjustments in other editors, and missing globally too. Each compares a
pixel with a blurred copy of its neighbourhood, so they need a multi-scale blur (or a
precomputed blurred pyramid) rather than a per-pixel formula.

**Plan**
- [ ] Research how they're usually built (local contrast at fine / medium scale; dehaze via
  a dark-channel or atmospheric-light estimate) and pick methods that run on the GPU in the
  preview and match in export, tiled.
- [ ] Global sliders (Effects or a new Presence section) and the same three on masks.
- [ ] Watch preview speed: blurs at full resolution while zoomed, and at tile edges in export.

### 8. Leftovers from the code review
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
- 2026-11-10 — Code review fixes (24 commits, `999a6a6`).
- 2026-11-10 — White balance presets: Daylight, Cloudy, Shade, Tungsten, Fluorescent, Flash (`5c95747`).
