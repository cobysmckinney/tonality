# TODO

What is left to do on Tonality, grouped by kind of work. The list is not complete; new items are added as they come up.

- Tick an item (`- [x]`) in the commit that finishes it.
- Add a new item to the section it fits, or start a new section.
- When an item turns out to be several pieces of work, indent the pieces under it as their own checkboxes.

## Bugs

- [ ] **Found parts land on the wrong mask.** `addMaskPart` reads the chosen mask when the model finishes, not when it was asked. Choose Add → Subject on mask A, click mask B during "Finding…", and the subject goes onto B. New mask or Esc during the wait drops it.
  - [ ] "Draw again" writes the old mask's part into whichever mask is chosen now (`store.ts` `updateMaskPart`).
- [ ] **Asking for a part while another is being found does nothing.** `foundFirst` returns silently, so a Ctrl+V of edits with subject or sky masks is lost without a toast, and a loop drawn meanwhile is thrown away.
- [ ] **One part that can't be found stops the photo drawing.** `draw_frame` and `open_editor` pass the error on, so the editor, thumbnail and export all fail and the mask can't be removed. Pasting an object loop onto a much smaller photo is enough ("Circle a larger part of the photo").
- [ ] **A RAW that makes rawler panic breaks editing until restart.** The panic poisons the `editing` mutex in `open_editor`; every later lock unwraps the poison. Thumbnail redraws run the same develop on the rayon pool with no guard, which aborts the app. `media.rs` already wraps the thumbnail decode in `catch_unwind`; the editor, thumbnail and export paths need the same.
- [ ] **Esc in the middle of a mask drag or brush stroke** leaves the drag live: after choosing another mask, plain hover writes the old part into it. Pressing C, M or Esc mid-drag in the crop tool leaves the drag uncommitted too.
- [ ] **Photos hidden by the flag filter stay selected.** With "Hide rejected", select a photo and press X: it disappears but stays selected, so Delete, Ctrl+E and F act on it and the arrow keys stop working.
- [ ] **After Delete in the grid, the arrow keys jump to the first photo** (`removeFromView` sets the cursor to `openId`, which is null in the grid).
- [ ] **Clarity darkens dehazed photos.** Clarity compares the dehazed pixel with a blur that was never dehazed (`develop.wgsl`), so positive clarity darkens hazy areas. Shadows and highlights read the same undehazed blur.
- [ ] **Undo doesn't undo the crop shape.** Pick 1:1, Ctrl+Z: the crop returns to 3:2 but 1:1 stays chosen, and the next corner drag snaps square.
- [ ] **Holding `\` at 100% on a cropped photo** shows a different spot, as the view is kept as a fraction of the cropped frame.
- [ ] **Double-clicking a slider's track** leaves an extra step: undo after the reset lands on the clicked value.
- [ ] **Dropping files while the export sheet is open** stacks the import sheet on it, and Esc closes both.
- [ ] **If one of the startup calls fails**, `init()` never subscribes to the progress and card events: progress bars stay at 0 and cards aren't noticed.
- [ ] **A deleted branch's name is given out again**: delete "Branch 2" of three and the next branch is a second "Branch 3".
- [ ] **Exported shutter speeds** of 1/4 s and 1/3 s are both written as 0.3 s.
- [ ] **Smaller ones.**
  - [ ] Double-clicking a mask to rename it also toggles it off.
  - [ ] Z zooms behind an open menu or dialog.
  - [ ] The vignette sits differently while cropping than after.
  - [ ] A mask's sharpening slider below 0 does nothing unless global sharpening is on.
  - [ ] Masks' exposure isn't capped once added together; overlapping masks can reach +10 EV.
  - [ ] "Original" crop ratio ignores a portrait crop's orientation.
  - [ ] A failed JPEG write leaves its `.part` file behind.

## Finish what's started

- [x] **Presets.** The only item the README still lists as missing.
- [ ] **Settings screen.** The library is fixed at `~/Pictures/Tonality` (only `TONALITY_LIBRARY` moves it), and nothing else is remembered: thumbnail size, the filter, the window's size and place.
  - [ ] The library's location has to live outside the library, in the app's config folder.
  - [ ] Exports are recorded by absolute path, so moving the library breaks History's "show file" for the default `Exports` folder.
- [ ] **Crop tool.**
  - [ ] Zoom while cropping.
  - [ ] Custom ratio.
  - [x] The Info tab still shows the uncropped size.
  - [ ] A way to cancel: Esc and Enter both mean Done, so only undo goes back.
- [ ] **Recently Deleted.** Expired photos are only removed at startup; in a long session they sit at "0 days left". Nothing tests that an expired photo is purged.
- [ ] **Export in the background.** It still blocks behind a dialog; it could use the title bar indicator instead. The backend already runs it off the main thread with progress events and a cancel flag, so this is mostly interface work, but:
  - [ ] `runExport` goes through the editor's queue (`inTurn`), so a background export would hold up opening and saving.
  - [ ] Exporting the photo on screen renders at full size while holding the editor's lock, freezing the sliders.
  - [ ] A single-file export has no Stop button.
- [ ] **Import.**
  - [ ] A scan can't be cancelled; dropping a huge folder by mistake locks the app until it ends.
  - [ ] Files and folders the scan can't read are skipped without a word.
  - [ ] Only the RAW of a RAW+JPEG pair is fingerprinted, so importing the JPEG on its own later brings it in again.
  - [ ] In-camera ratings (EXIF, XMP sidecars) are ignored.
  - [ ] A camera rawler doesn't know imports and looks fine in the grid (its JPEG), then fails in the editor. Say "unsupported camera" at import.
- [ ] **Albums.** No dragging photos onto an album, sorted only by name, no cover. "New album" writes "Untitled album" before it is named, so Esc leaves it behind.

## Image quality

- [ ] **Colour management.** Everything is assumed sRGB.
  - [ ] Embedded ICC profiles are ignored, in the editor and in thumbnails, so Display P3 and Adobe RGB sources look slightly off.
  - [ ] The working space is linear sRGB, and rawler's Calibrate step clips colours outside it when the RAW is loaded (saturated blues, flowers).
  - [ ] Exports carry no ICC profile, only EXIF `ColorSpace=1`.
  - [ ] The monitor profile isn't used.
- [ ] **White balance in Kelvin, with an eyedropper.** Today it is relative to as-shot; Kelvin needs camera-space data the backend doesn't keep. DNGs with neither WB coefficients nor `AsShotWhiteXY` open green.
- [ ] **Highlight recovery** past the sensor's clipping point. There is no headroom at all: rawler's white balance pushes unclipped red and blue past 1.0 and Calibrate squashes them before the shader sees them.
- [ ] **Sensor support.**
  - [ ] Fuji X-Trans gets rawler's bilinear demosaic; it has a Markesteijn one.
  - [ ] Four-colour sensors are untested. Calibrate probably turns them three-colour before `develop.rs` can refuse them; one without a D65 matrix makes rawler panic.
- [ ] **Dehaze** is too strong near +100 (a gain of about 2.2×), and grows with exposure.
- [ ] **Noise reduction and sharpening** are simple first versions: one 5×5 bilateral filter on luma with no colour noise control, and a 4-neighbour sharpen with no radius, detail or masking. Both read the mip level for the zoom, so fit view differs from a full-size export.
- [ ] **Unedited thumbnails** come from the camera's JPEG, so the look shifts slightly on first edit. A RAW+JPEG pair whose JPEG is cropped or black and white shows that shape or look in the grid, and a small embedded preview falls back to rawler's default develop, a third look.
- [ ] **Very large photos are halved** until they fit the GPU's largest texture, not scaled to fit, and a "full size" export comes out halved without saying so. On 16,384 px a 20,000 px panorama becomes 10,000; on Android's 8,192, a 9,000 px photo becomes 4,500.
- [ ] **Mask detail stops at 3072 px** (`COVERAGE_EDGE`). On a 45 MP photo at 100%, brush and found edges are stretched about 2.7×.
- [ ] **Deep shadows may band** at large pushes, as the f16 source texture goes subnormal near zero (unverified).

## Speed and memory

- [ ] **Opening a RAW.** About a second every time; nothing is cached. The same full develop runs for grid paste, exporting a photo that isn't open, and redrawing a missing thumbnail.
- [ ] **Finding the subject.** IS-Net takes about six seconds on the CPU the first time for each photo; sky runs two models.
  - [ ] Grid paste, thumbnail redraw and export of photos with pasted subject or sky masks run the models for each photo, inside the render path.
  - [ ] `find_parts` doesn't take the session's `finding` lock, so it can repeat work a frame is already doing.
  - [ ] Cached mattes are keyed only by model, so a decoder or orientation change leaves stale ones.
- [ ] **Every frame** makes a new target texture and readback buffer, and renders again at 256 px for the histogram.
- [ ] **Memory on open** is roughly 1.3 GB for 45 MP: an f32 image, an f16 copy and the f16 texture with mips, plus another full copy while rotating.
- [ ] **Mask thumbnails** are redrawn after every slider change, even ones that can't change a mask.
- [ ] **Display frames stop at 4096 px**, so a 5K screen at 2× gets an upscaled frame.

## Holding up

- [ ] **No error screens.** If the library can't open at startup (Pictures not writable, a locked database), the app panics with no dialog. The interface has no React error boundary, so a render error leaves an empty window.
- [ ] **One instance only.** Launching twice runs two processes on the same `library.db`.
- [ ] **Library integrity.** Purging deletes the database row before the file and ignores failures, which can leave orphan originals. Nothing notices originals moved or deleted in a file manager. Import copies are checked by size only and not synced.
- [ ] **Flags and favourites stay in the app**: they are never written to XMP or exports.

## Features not started

- [x] **Local adjustments**: masks, gradients, brushes.
  - [ ] Color range masks, picked from the photo. The brightness range has no eyedropper either.
  - [x] Subject and background masks found automatically.
  - [x] Sky masks found automatically, and objects selected by drawing a loop.
  - [x] Sky seen through tree canopies: the small sky model misses blown-out sky between branches.
  - [ ] Keyboard nudges for a mask's handles.
  - [ ] Painting or drawing a loop while zoomed in can't pan: both capture the drag, and there is no Space-drag or middle-button pan. Only the scroll wheel works.
- [ ] **Spot removal.**
- [ ] **Lens corrections and perspective.** The geometry is affine; perspective needs a projective transform in `geometry.rs` and the shader.
- [ ] **Search, sorting and filtering** by camera, lens or date. Sort orders are fixed in SQL, the grid's items carry no camera or lens, and the only filter is by flag, which resets on every view change. Each view loads the whole library.
- [ ] **Keywords, captions and star ratings.**
- [ ] **Side-by-side compare.** Today there is only holding `\`.
- [ ] **Shortcuts reference.** J, O, `[` / `]`, Ctrl+Y and Backspace (which deletes the photo) are only in tooltips; the grid's right-click menu shows no keys at all.

## Keyboard and accessibility

- [ ] **The crop frame and tone curve** can't be used from the keyboard.
- [ ] **Preset tiles' star and "…" buttons** are skipped by Tab, so favouriting, renaming, updating, exporting and deleting a preset need a mouse.
- [ ] **Tab lists** (tool rail, mixer bands, curve channels) say `role="tablist"` but Left and Right switch photos, and none has `aria-controls`.
- [ ] **Show original** works only by pointer and `\`; Space and Enter do nothing.
- [ ] **Backspace or Delete with any button focused** sends the open photo to Recently Deleted. There is an undo toast, but it is easy to do by accident.

## Tests

- [ ] **Shader.** Nothing checks highlights, shadows, whites, blacks, contrast, clarity, dehaze, temperature, tint, vibrance, vignette, grain, noise reduction, sharpening, the RAW base look, or straighten and masks drawn through the shader.
- [ ] **Backend.** Expired photos being purged, schema upgrades, `volumes.rs` mount parsing, `uncompress_jpeg_tiles` and the `AsShotWhiteXY` path, cancelling an import, exporting a photo with subject or sky masks that isn't open, `shrink_to_fit`.
- [ ] **Interface.** `describeChange`, `curveSampler` (and that it matches the backend's `curve_table`), the mask limits, and the zoom and pan arithmetic inline in `Editor.tsx`.
- [ ] **The README's `bun test` line** doesn't mention `masks.test.ts`.

## Shipping

- [ ] **A release build.** `bun run tauri build` has never been run, so the installer and launcher icon are unverified. With 145 MB of models compiled in and `lto = true`, expect a slow link and a large binary; shipping the models as resources, or downloading them on first use, would help.
- [ ] **Bundle metadata.** No publisher, copyright, category, descriptions, homepage or licence in `tauri.conf.json`; `authors = ["you"]` in `Cargo.toml`. The `.deb` and `.rpm` don't depend on libheif, which is linked dynamically.
- [ ] **Security.** `csp` is `null`, and commands take arbitrary paths (preset export and import, scan, the export folder). Set a Content Security Policy and drop the unused `opener` `open-url` permission.
- [ ] **macOS and Windows** are untested.
  - [ ] The window controls would need a macOS layout (`titleBarStyle: "Overlay"`).
  - [ ] Shortcut labels say "Ctrl" everywhere.
  - [ ] `heif` is a default feature, so a default build needs libheif from vcpkg or Homebrew.
- [ ] **Android.** The only debug APK (x86_64, 584 MB) was built before the models came in, so whether it still builds is unknown, and the README doesn't mention it.
  - [ ] The interface leans on hover, right-click and keyboard shortcuts.
  - [ ] The manifest limits it to tablets (`requiresSmallestWidthDp="600"`), it asks for no media permission, and release signing isn't set up.
  - [ ] The models are in every ABI's library; GPUs with an 8,192 px limit halve most photos (see Image quality).
  - [ ] Check that `picture_dir()` means anything there.
- [ ] **A remote**, then CI to run the tests on push. CI will need libheif, Bun and the models.
  - [ ] Decide on Git LFS for the models first: they are 145 MB of plain blobs, and `isnet_general_use.f16.onnx` (90.6 MB) is near GitHub's 100 MB limit. Every swapped model stays in history.
- [ ] **A licence.** rawler is LGPL-2.1 and statically linked, which constrains the choice.
  - [ ] Ship third-party notices: the model licences (Apache-2.0, MIT, BSD-3) and rembg's sit only in `src-tauri/models`, and nothing carries the crate, npm or Geist (OFL) notices. An About screen, and a generator such as cargo-about.
  - [ ] Check whether the models' training data (DIS5K, SA-1B) limits commercial use.
