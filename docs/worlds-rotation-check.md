# Worlds rotation check — 2026-09-22

The current user's three saved worlds were loaded in the macOS Tauri development
app. A temporary binary frame counter was painted after each actual WebGL draw.
Ten-second, 60 fps screen recordings were decoded with AVFoundation. Analysis
excluded the first and last 30 captured frames; the center molecule's pixels were
also compared to distinguish counter repetition from content motion.

| Version | Interior captured frames | Repeated frame IDs | Skips of one ID |
| --- | ---: | ---: | ---: |
| Original (separate update/draw callbacks, averaged timestep) | 529 | 66 | 66 |
| Update and draw in one callback, elapsed-time integration | 529 | 23 | 23 |
| Also disable default-framebuffer MSAA and flush completed draws | 530 | 12 | 12 |

The original CPU draw submission took about 0.45 ms on average, with a maximum
of 1 ms in the sampled 600-frame block. Draw callbacks averaged 16.67 ms with no
gap above 25 ms, despite the repeated frames in the recording. Counting callbacks
alone therefore did not establish smooth presentation.

These are measurements of the screen-recording path, which itself can introduce
sampling and encoding effects; they do not prove that every remaining repeated
frame was visible on the physical display or that no judder remains. The measured
repetition fell by about 82% under these capture conditions.

The implementation retains the ribbon geometry and offscreen antialiasing. It
updates the pose immediately before drawing, integrates the turn easing over the
actual elapsed interval, removes a redundant multisample resolve of the final
fullscreen quad, and flushes commands without waiting for GPU completion.

The temporary counter, automatic navigation, and diagnostic logs were removed.
Four scheduler regression tests cover current-frame poses, coalesced invalidations,
replacement animations, stopping on screen detach, resuming, and idle rendering.
The frontend suite passes 92 tests.
