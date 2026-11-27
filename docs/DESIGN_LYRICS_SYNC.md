# Design: Synced lyrics with fade (`l` popup, v2)

## Goal
The lyrics popup becomes a **karaoke-follow view**: the line currently singing
stays centered and bright, nearby lines fade progressively (color blend toward
the background), and the view auto-scrolls as playback advances. Untimed
lyrics keep the current plain, scrollable rendering.

## Data (already in the tags — zero network)
The FLAC `LYRICS` tag is LRC with **line timestamps** (`[mm:ss.xxx]`, often
several per line for repeats) — currently discarded by `clean_lyrics`.

- `parse_lrc(raw) -> Option<LrcLines>`: for each line, take **every** leading
  `[mm:ss(.xxx)]` timestamp (a line with N timestamps repeats N times), strip
  inline word-timestamps `<…>` and voice labels (same bracket-state logic as
  `clean_lyrics`), trim. Returns `None` when the tag has no line timestamps
  at all → plain fallback.
- App state: `lyrics: Option<Lyrics>` with
  `enum Lyrics { Timed(LrcLines), Plain(String) }`,
  `LrcLines { lines: Vec<LrcLine> }`, `LrcLine { time: f64, text: String }`
  sorted by time.

## Follow + fade
- Active line = last line with `time <= time_pos + 0.3` (small lead feels
  right; `time_pos` comes from the existing status poll).
- The active line renders centered in the popup; `j`/`k` apply a manual
  offset (rows away from center) that **resets to 0 when the active line
  changes** — browse freely, the view re-follows on its own. `g` = jump the
  offset so the first line is visible.
- Fade = per-line fg color: blend `Rgb(255,255,255)` (active) toward
  `Rgb(60,60,60)` as distance from center grows (capped at 5 lines). No
  background blending — the user's terminal bg stays theirs.
- Active line styled with the popup accent (same ACCENT_2 family as borders).
- `time_pos` absent (nothing playing / mpv not reporting): show the full list
  starting at the top, no highlight (graceful, like today).
- `Plain` fallback renders exactly as today (scrollable paragraph).

## Non-goals (v1 of this feature)
- No word-level karaoke (the `<mm:ss>` word tags exist but are duplicated and
  fragile — line level reads perfectly).
- No smooth animation/tweening between lines (terminal grid; per-tick update
  is enough).
- No transliteration changes (existing `t` toggle keeps working on the
  cleaned text).

## Testing (TDD)
- `parse_lrc`: multi-timestamp repeat lines, word-timestamps + voice labels
  stripped, metadata headers dropped, no-timestamp → None, unsorted input
  sorted by time.
- UI (TestBackend): timed popup centers the active line (computed from a
  fake `time_pos`), active bright vs distant dim (fg colors asserted on the
  buffer), manual offset shifts the window, Plain unchanged, empty-safe on
  tiny areas.
- App: parse path via fake player metadata, fallback to Plain when no
  timestamps.
