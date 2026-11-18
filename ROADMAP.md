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

_Nothing queued. Pick the next item from **Later** or add a new one._

---

## Before a commercial release
- [ ] Replace SegFormer-B0 (sky/people masks): its license is non-commercial only.

## Later
_(new ideas go here)_
- AgX for Display P3 (and HDR) output: swap the display matrix and guard-rail weights (`docs/agx.md`).

## Done
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
