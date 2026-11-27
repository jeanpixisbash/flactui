# Design: Cover popup (`i`)

## Goal
A dedicated view (like the lyrics popup) that shows the **embedded album art
of the playing/selected track, rendered large** with Unicode half-blocks and
truecolor — works in alacritty (no sixel/kitty protocol needed).

## UX
- Key `i` in library or queue view → popup modal, centered, ~60% of the
  terminal (same family as the lyrics popup: box, rounded border).
- Border title: ` cover · <track name> ` (truncated).
- No picture in the tag → popup shows `no embedded cover` (still a popup, so
  the key always does something visible).
- Close: `Esc`, `i`, `q` (consistent with lyrics popup).
- While loading (read + decode over FUSE can take ~200-500 ms): message
  `loading cover…` is already handled by the existing status bar.

## Data path
1. Take the path of the track (playing one if any, else the selected one).
2. Extract the FLAC `METADATA_BLOCK_PICTURE` by hand (std-only parser in
   `covers.rs`; **plan deviation, decided during planning:** the `lofty`
   crate was dropped — the hand parser is ~40 lines, testable with a
   synthetic fixture, and avoids a heavy dep tree. Consequence: **FLAC
   covers only in v1** — m4a shows `no embedded cover`; 13 of 591 library
   tracks). Only the metadata prefix is read, never the audio stream.
3. Cache the extracted picture bytes in `~/.cache/flactui/covers/<hash of
   path>.img` — subsequent opens are instant, no FUSE re-read.
4. Decode with `image` (jpeg+png only), resize to the fixed grid
   (48×24 cells = 48×48 px, Triangle filter), render one half-block per
   cell: top pixel = fg, bottom pixel = bg, char `▀`. The popup center-crops
   the fixed grid every frame (cheap), so no re-resize on terminal resize.

## Render rule (ratatui)
For each cell: `Style::new().fg(top_rgb).bg(bottom_rgb)`, symbol `▀`.
Popup area smaller than 4×4 → skip drawing (avoid division by zero).
Superseded by the amended rule above (fixed grid, letterboxed).

## Failure modes
- Tag without picture → `no embedded cover` text in popup.
- Unreadable file / decode error → popup shows `cover unavailable` +
  reason in the existing status message. Never crashes the TUI.
- Cache dir not writable → proceed without cache (regenerate each time).

## Non-goals (v1)
- No permanent mini-cover in the playbar (later if wanted).
- No sixel/kitty/ueberzug real-image rendering.
- No cropping/scaling options; fit is always "fill popup, center-crop".
- No fetching covers from the internet — embedded tags only (same policy as
  lyrics).

## Render rule (ratatui) — amended after review
The grid is FIXED at 48×24 cells (48×48 px). The popup centers it: on large
terminals it is letterboxed (not "filled"), on small ones it is
center-cropped. No re-resize on terminal resize (cache-friendly).

## Testing (TDD)
- `covers.rs` unit tests: extract from a tiny generated FLAC/PNG fixture;
  cache write/reuse; resize math; pixel→half-block conversion.
- `ui.rs` TestBackend test: popup renders over the library, shows `▀`
  symbols and the title; closes with `Esc`.
- `app.rs` tests with a fake path → `no embedded cover` path.
