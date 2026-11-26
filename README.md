# flactui

> **A spotify-tui-style TUI for your phone's music library.** Browse and play the
> music stored on your phone from a Linux terminal — no re-downloading, no media
> server. Powered by KDE Connect + mpv. One static Rust binary.

```text
┌─ flactui ───────────────────────────────────────────────────────┐
│  library (591)                        queue (3)                 │
│ ▸ 01 - Canserbero - Es Épico.flac    ▶ Es Épico.flac           │
│   02 - Canserbero - Vida.flac          Vida.flac                │
│   03 - Silvestre Dangond - ...         Perdon.flac              │
├─────────────────────────────────────────────────────────────────┤
│ ▶ 01 - Canserbero - Es Épico.flac   3:12/5:40                  │
│ ━━━━━━━━━━━━━━━━○─────────────────────                         │
│ j/k move · enter play · space queue · / search  vol:100%  q quit│
└─────────────────────────────────────────────────────────────────┘
```

Your phone keeps its FLAC (or whatever) library; your computer has the good
speakers. `flactui` mounts the phone over KDE Connect's SFTP share, indexes it
locally (cached — the picker opens instantly) and streams it into a persistent
mpv controlled over IPC, so **you keep browsing while the music plays**.

## Sibling project

Prefer a lighter tool? **[flacast](https://github.com/jeanpixisbash/flacast)**
is the bash + fzf CLI version of the same idea — same plumbing, terminal
minimalist UX.

## Requirements

- Linux
- [KDE Connect](https://kdeconnect.kde.org/) installed and **paired**
  (phone app + desktop; the daemon must be running)
- `mpv` on your PATH
- That's it: D-Bus calls are made natively (zbus), no `kdeconnect-cli` needed

## Install

**From source** (needs the Rust toolchain):

```bash
git clone https://github.com/jeanpixisbash/flactui
cd flactui
./install.sh        # builds with cargo, copies to ~/.local/bin (no sudo)
```

**Or grab a prebuilt binary** from
[GitHub Releases](https://github.com/jeanpixisbash/flactui/releases)
(`x86_64-unknown-linux-musl`, statically linked — drop it in your `PATH`).

## Usage

```bash
flactui                  # the TUI
flactui --keep-mounted   # skip the unmount prompt on exit
```

| Keys | Action |
|---|---|
| `j` / `k` (or arrows) | move cursor |
| `Enter` | play selection |
| `Space` | enqueue selection (in the queue view: play/pause) |
| `a` | enqueue everything currently filtered |
| `/` | fuzzy search (type, `Enter`/`Esc` to end) |
| `Tab` | switch library ↔ queue view |
| `n` / `p` | next / previous track (both views) |
| `←` / `→` | seek −10 s / +10 s |
| `s` / `r` | toggle shuffle / repeat |
| `+` / `-` | volume (both views) |
| `q` | quit (offers to unmount the phone) |

In the queue view: `d` removes a track, `J`/`K` move it down/up, `Enter` jumps
to it, `D` clears the queue.

## How it works

1. Finds a reachable paired phone over D-Bus (`org.kde.kdeconnect.daemon`).
   If the session runs on a foreign D-Bus (`dbus-launch`), it retries on the
   systemd user bus automatically.
2. Mounts the phone with the KDE Connect SFTP plugin
   (`org.kde.kdeconnect.device.sftp`).
3. Indexes the music folder to `~/.cache/flactui/library.list` (rebuilds when
   missing; delete it after adding music to the phone).
4. Spawns one persistent `mpv --idle` and drives it over its JSON IPC socket
   (`loadfile`, playlist ops, volume, seek) — the TUI never blocks on playback.

## Troubleshooting

| Symptom | Fix |
|---|---|
| *"no reachable phone found"* | open the KDE Connect app on the phone (Android kills it in the background); check both devices are on the same network |
| *"no audio files found"* | your phone keeps music elsewhere → `export FLACAST_PHONE_MUSIC_DIR="storage/emulated/0/Your/Music"` |
| stale library after adding songs | `rm ~/.cache/flactui/library.list` and restart |
| phone paired but discovery fails | on some firewalls, allow the KDE Connect ports: `sudo ufw allow 1714:1764/tcp && sudo ufw allow 1714:1764/udp` |

## AI assistance

flactui is developed with the help of an AI coding agent (OpenCode) under human
direction: the idea, the design decisions, testing on real hardware and every
release are human-approved. The codebase is small and auditable. AI-assisted
contributions are welcome, but please disclose their use in your pull request;
commits made with AI help carry an `Assisted-by:` trailer.

## Español (resumen)

`flactui` es una TUI estilo spotify-tui pa' escuchar la música de tu celular en
tu PC **sin volver a descargarla**: monta el celu por KDE Connect (SFTP), indexa
la biblioteca en caché y reproduce con un mpv persistente controlado por IPC —
sigues navegando mientras suena. Un solo binario Rust, sin sudo. Hermano CLI:
[flacast](https://github.com/jeanpixisbash/flacast).

> Transparencia: el proyecto se desarrolla con ayuda de un agente de IA bajo
> dirección humana — la idea, las decisiones de diseño, las pruebas en hardware
> real y cada release son aprobados por una persona. PRs con IA bienvenidos si
> se declaran.

## License

[MIT](LICENSE) — © 2026 Jean Pierre Trujillo Wilches
