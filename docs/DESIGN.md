# flactui — Design

## Problem

Same as flacast: people keep their real music library (often FLAC) on their
phone. A full-screen terminal UI should make it pleasant to browse and play
that library from a Linux desktop — in the spirit of
[spotify-tui](https://github.com/Rigellute/spotify-tui) — while playback
keeps running while you browse.

`flacast` (the sibling project, bash + fzf) stays as the lightweight CLI;
`flactui` is the TUI for people who want the full-screen experience.

## Decision log (user-approved 2026-11-26)

- **Language: Rust + ratatui** (user choice A). Single static binary, no
  runtime deps beyond mpv itself. This is the same stack as spotify-tui.
- **Separate project** from flacast (two repos, cross-linked READMEs).
- **Playback via persistent mpv + IPC socket** (JSON over unix socket,
  the `--input-ipc-server` protocol). The TUI never blocks on playback:
  browse, queue, seek and adjust volume while music plays.
- **v1 scope:** single screen — library browser on top (cache-backed,
  fuzzy search with `/`, vim keys), now-playing playbar at the bottom
  (progress, transport, shuffle/repeat, volume) and a queue view
  (reorder/clear).

## Architecture

```text
┌────────────────────── flactui (Rust) ──────────────────────┐
│  ui (ratatui)                                              │
│    library pane ─ queue pane ─ playbar                     │
│  app state (selected track, queue, status events)          │
│  ────────────────────────────────────────────────────────  │
│  kde.rs   zbus: org.kde.kdeconnect.device.sftp             │
│           find reachable device → mount → mount point      │
│           (falls back to the systemd user bus, like flacast)│
│  cache.rs library listing at ~/.cache/flactui/library.list │
│  player.rs mpv spawned as child with --input-ipc-server;   │
│           JSON IPC: loadfile, playlist ops, volume, seek;  │
│           property events (pos, pause, metadata) feed the UI│
└────────────────────────────────────────────────────────────┘
```

- Library listing = file paths from the mounted phone (same as flacast;
  filenames carry artist/title, no tag parsing in v1).
- mpv is spawned once per session; on quit, the TUI asks to unmount the
  phone (same farewell UX as flacast) and kills mpv.

## Crates (first pass)

`ratatui` + `crossterm` (UI), `tokio` (async runtime), `zbus` (D-Bus),
`serde`/`serde_json` (IPC), `dirs` (paths). No libmpv binding — mpv is an
external process, keeping the binary static and the build simple.

## Non-goals (v1)

- No tag parsing / embedded art (filenames only).
- No writing to the phone; no multi-device.
- No config file yet (keymap and theme hardcoded spotify-tui-ish; config
  file is a later addition).

## Testing

- Unit tests for cache handling, playlist building, and IPC message
  framing against a scripted fake mpv socket (no audio, no phone).
- One integration test: spawn a fake IPC server, drive the app loop,
  assert state transitions (enqueue, next, pause).
- Real-device verification is manual and bounded (real phone, `timeout`).

## Packaging

- `cargo build --release` → one binary; `install.sh` copies it to
  `~/.local/bin` (no sudo), same installer UX as flacast.
- GitHub Releases: binaries for x86_64-linux (musl for max portability).
