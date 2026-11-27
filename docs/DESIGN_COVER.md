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
2. Read the tag with `lofty` (default-features off; only `flac` + `m4a`
   features) → embedded picture bytes (`METADATA_BLOCK_PICTURE` / `covr`).
3. Cache the extracted picture bytes in `~/.cache/flactui/covers/<blake3-ish
   hash of path>.<ext>` — subsequent opens are instant, no FUSE re-read.
4. Decode with `image` (jpeg+png only), resize to the popup's pixel size
   (`cols × (rows*2)`, Triangle filter), render one half-block per cell:
   top pixel = fg, bottom pixel = bg, char `▀`.

## Render rule (ratatui)
For each cell: `Style::new().fg(top_rgb).bg(bottom_rgb)`, symbol `▀`.
Popup area smaller than 4×4 → skip drawing (avoid division by zero).

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

## Testing (TDD)
- `covers.rs` unit tests: extract from a tiny generated FLAC/PNG fixture;
  cache write/reuse; resize math; pixel→half-block conversion.
- `ui.rs` TestBackend test: popup renders over the library, shows `▀`
  symbols and the title; closes with `Esc`.
- `app.rs` tests with a fake path → `no embedded cover` path.
