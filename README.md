# RTunes

Terminal music player with **10 audio-reactive visualizers**, a Ratatui library UI, and optional **yt-dlp** downloads. Built in Rust; runs on Windows, macOS, and Linux.

*(Optional: add a screenshot under `docs/` and link it here.)*

## Install

### From source

```bash
cargo install --path .
# or
cargo build --release
# binary: target/release/rtunes (target/release/rtunes.exe on Windows)
```

### Binary releases

See the `dist/` output from [`scripts/package.ps1`](scripts/package.ps1) / [`scripts/package.sh`](scripts/package.sh) after `cargo build --release`.

## Quick start

```bash
rtunes library add ~/Music
rtunes tui
```

Use `/` to search, **Space** to play/pause, **v** / **V** to cycle visualizers, **d** to paste a download URL.

## `deps/` folder (yt-dlp + ffmpeg)

Place **`yt-dlp`** and **`ffmpeg`** next to the executable under `deps/`, or install them on your **PATH**. RTunes resolves `PATH` first, then `<exe_dir>/deps/`.

On Windows: `deps\yt-dlp.exe`, `deps\ffmpeg.exe`.  
Configure explicit paths in `config.yaml` under `fetcher.ytdlp_path` / `fetcher.ffmpeg_path` (`"auto"` = search).

## Commands

| Command | Description |
|--------|---------------|
| `rtunes tui` | Full-screen TUI (default experience) |
| `rtunes fetch <URL>` | One-shot download with progress on stderr |
| `rtunes scan` | Scan configured library paths |
| `rtunes library add \| remove \| list` | Manage `library_paths` in config |

Global flags: `--config <path>`, `--log-level <filter>` (e.g. `info`, `debug`).

## Keybindings (TUI)

| Keys | Action |
|------|--------|
| **Space** | Play / pause |
| **n** / **p** | Next / previous track |
| **←** / **→** | Seek ±5s (Shift: ±30s) |
| **+** / **-** | Volume |
| **m** | Mute |
| **s** | Shuffle |
| **r** | Repeat cycle |
| **↑** / **↓** / **j** / **k** | Library cursor |
| **Enter** | Play selected |
| **/** | Search |
| **d** | Download URL prompt |
| **a** | Add library folder prompt |
| **Ctrl+L** | Library manager |
| **v** / **V** | Cycle visualizer |
| **1**–**9**, **0** | Jump to visualizer |
| **t** | Cycle theme |
| **g** | Neon (glow) toggle |
| **Shift+M** | Spectrogram layout (when Spectrogram is active) |
| **f** | Fullscreen layout |
| **?** | Help overlay |
| **Esc** | Close overlay / quit (from normal mode) |
| **q** | Quit |

## Configuration

On first run, RTunes creates a YAML config under the OS config directory (override with **`RTUNES_CONFIG_PATH`** or **`rtunes --config`**).

Authoritative defaults and field descriptions live in **[`assets/default_config.yaml`](assets/default_config.yaml)** — copy values into your user config as needed.

## Logging

File logs (daily rotation) under the platform **local data** directory, e.g. Windows: `%LOCALAPPDATA%\rtunes\`. Override verbosity with `RTUNES_LOG_LEVEL` or `--log-level`.

## Development

```bash
cargo test
cargo test --release   # stricter scan perf budget for 1000-track fixture
```

## License

Dual-licensed under **MIT OR Apache-2.0** (see `Cargo.toml`).
