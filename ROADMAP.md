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

### 1. Leftovers from the code review
**Status:** done on branch `leftovers`; waiting for Coby's review.
- [x] Tried in the app (scripted): no content-security-policy violations or script errors;
  the library thumbnail is saved after leaving Edit right after an edit; stepping quickly
  through photos with masks ends on the right photo and edit. "Fluorescent" is CIE F2 (cool
  white, ≈4230 K), as it should be.
- [x] Values below black are kept: the decoder no longer clips them (rawler's scaling did), noise
  reduction averages them (luminance linearly, weighted in √L; chroma as differences), and black
  is clipped after. A GPU test: a noisy patch around black stays within ~15% of the clean one,
  against +45% when clipped first. (Sony and Nikon clip at black in camera; Canon doesn't.)
- [x] Full-resolution prefetch decodes on a low-priority pool of their own, so a zoom waiting
  for one doesn't also wait behind AI detection or thumbnails.
- [x] Catalog v4: HEAD must name one of the photo's branches (foreign key, deferred; renames
  carry HEAD). The migration repairs a dangling HEAD first; tried on a copy of Coby's catalog.

---

## Before a commercial release
- [ ] Replace SegFormer-B0 (sky/people masks): its license is non-commercial only.

## Later
_(new ideas go here)_
- AgX for Display P3 (and HDR) output: swap the display matrix and guard-rail weights (`docs/agx.md`).

## Done
- 2026-11-17 — Texture, Clarity, Dehaze in Effects and on masks (`presence`, `9b08681`; `docs/presence.md`).
- 2026-11-17 — AgX done properly: Blender's AgX Base from its parameters, checked against Blender's LUTs (`agx`, `8237c3b`; `docs/agx.md`).
- 2026-11-17 — Reset history, from Edit or for selected photos in the Library (`history-reset`). Cleaned the real library of 9 duplicate branches left by the old sidecar bug (backup in `~/Pictures/tonality-backup-2026-11-17-before-dedupe`), and fixed the merge so a copy in today's format can't capture the original.
- 2026-11-17 — Logo: italic Playfair `t` mark and wordmark, app icons, startup animation (`logo`, `e22ba27`).
- 2026-11-17 — Crop: aspect presets as icon buttons, orientation switch, 3:4; sidecar merge no longer duplicates histories when a new setting is added (`crop-buttons`, `7fbec47`).
- 2026-11-17 — Library: justified rows, photos edge to edge in their own shape; catalog v3 records each photo's shape (`library-cards`).
- 2026-11-17 — Masks rebuilt: floating list with black-and-white mask thumbnails (bottom right), settings in the side panel, red overlay on by default, hover to preview; local Hue, Vibrance, Color Tint, Sharpness and Noise Reduction (`masks-ui`).
- 2026-11-10 — Code review fixes (24 commits, `999a6a6`).
- 2026-11-10 — White balance presets: Daylight, Cloudy, Shade, Tungsten, Fluorescent, Flash (`5c95747`).
