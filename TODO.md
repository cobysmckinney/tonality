# TODO

What is left to do on Tonality, grouped by kind of work. The list is not complete; new items are added as they come up.

- Tick an item (`- [x]`) in the commit that finishes it.
- Add a new item to the section it fits, or start a new section.
- When an item turns out to be several pieces of work, indent the pieces under it as their own checkboxes.

## Finish what's started

- [x] **Presets.** The only item the README still lists as missing.
- [ ] **Settings screen.** The library is fixed at `~/Pictures/Tonality` with no way to move it.
- [ ] **Crop tool.**
  - [ ] Zoom while cropping.
  - [ ] Custom ratio.
  - [ ] The Info tab still shows the uncropped size.
- [ ] **Recently Deleted.** Expired photos are only removed at startup.
- [ ] **Opening a RAW.** About a second every time; nothing is cached.
- [ ] **Export in the background.** It still blocks behind a dialog; it could use the new title bar indicator instead.

## Image quality

- [ ] **Colour management.** Everything is assumed sRGB.
  - [ ] Display P3 and Adobe RGB sources look slightly off.
  - [ ] The monitor profile isn't used.
- [ ] **White balance in Kelvin, with an eyedropper.** Today it is relative to as-shot.
- [ ] **Highlight recovery** past the sensor's clipping point.
- [ ] **Sensor support.**
  - [ ] Fuji X-Trans gets only a basic demosaic.
  - [ ] Four-colour sensors aren't supported.
- [ ] **Dehaze** is too strong near +100.
- [ ] **Noise reduction and sharpening** are simple first versions.
- [ ] **Unedited thumbnails** come from the camera's JPEG, so the look shifts slightly on first edit.

## Features not started

- [x] **Local adjustments**: masks, gradients, brushes.
  - [ ] Color range masks, picked from the photo.
  - [x] Subject and background masks found automatically.
  - [x] Sky masks found automatically, and objects selected by drawing a loop.
  - [x] Sky seen through tree canopies: the small sky model misses blown-out sky between branches.
  - [ ] Keyboard nudges for a mask's handles; painting while zoomed in can't pan.
- [ ] **Spot removal.**
- [ ] **Lens corrections and perspective.**
- [ ] **Search, sorting and filtering** by camera, lens or date.
- [ ] **Keywords, captions and star ratings.**
- [ ] **Side-by-side compare.**
- [ ] **Shortcuts reference.**

## Shipping

- [ ] **A release build.** `bun run tauri build` has never been run, so the installer and launcher icon are unverified.
- [ ] **macOS and Windows** are untested; the window controls would need a macOS layout.
- [ ] **A remote**, then CI to run the tests on push.
- [ ] **A licence.**
