# RTunes

Terminal music player with **10 audio-reactive visualizers**, a Ratatui library UI, and optional **yt-dlp** downloads. Built in Rust; runs on Windows, macOS, and Linux.

## Install

### From source

**Linux** — install ALSA dev headers first:
```bash
# Debian / Ubuntu
sudo apt install libasound2-dev
# Fedora
sudo dnf install alsa-lib-devel
# Arch
sudo pacman -S alsa-lib
```

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

RTunes auto-downloads **yt-dlp** and **ffmpeg** on first use if they aren't found. They are saved to `deps/` beside the executable and reused automatically.

You can also install them manually:

| Platform | yt-dlp | ffmpeg |
|---|---|---|
| **Windows** | `winget install yt-dlp.yt-dlp` | `winget install Gyan.FFmpeg` |
| **macOS** | `brew install yt-dlp` | `brew install ffmpeg` |
| **Linux** | `sudo apt install yt-dlp` | `sudo apt install ffmpeg` |

Or set explicit paths in `config.yaml` under `fetcher.ytdlp_path` / `fetcher.ffmpeg_path` (`"auto"` = search `deps/` then PATH).

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

On first run, RTunes creates a YAML config under the OS standard config directory:

| Platform | Default config path |
|----------|--------------------|
| Windows  | `%APPDATA%\rtunes\config.yaml` |
| macOS    | `~/Library/Application Support/rtunes/config.yaml` |
| Linux    | `~/.config/rtunes/config.yaml` |

**Portable mode**: if `config.yaml` exists next to the `rtunes` binary (e.g. in a self-contained archive), it takes priority over the OS config directory.

Override entirely with **`RTUNES_CONFIG_PATH`** env var or **`rtunes --config <path>`**.

Authoritative defaults and field descriptions live in **[`assets/default_config.yaml`](assets/default_config.yaml)** — copy values into your user config as needed.

## Logging

File logs (daily rotation) under the platform **local data** directory, e.g. Windows: `%LOCALAPPDATA%\rtunes\`. Override verbosity with `RTUNES_LOG_LEVEL` or `--log-level`.

## Terminal compatibility

rtunes renders Unicode block characters and ANSI colours. Recommended terminals:

| Platform | Recommended |
|----------|------------|
| Windows  | [Windows Terminal](https://aka.ms/terminal), WezTerm, Alacritty |
| macOS    | iTerm2, Alacritty, WezTerm |
| Linux    | Any modern VTE/xterm terminal; Wayland compositors supported |

> **Note (Windows)**: legacy `cmd.exe` / ConHost may not render visualizer block characters correctly. Windows Terminal (bundled with Windows 11) is recommended.

## Development

```bash
cargo test
cargo test --release   # stricter scan perf budget for 1000-track fixture
```

The first `cargo test` run installs a pre-commit hook via `cargo-husky` that runs `cargo fmt --check` and `cargo clippy` before every commit. No manual setup needed.

## License

Dual-licensed under **MIT OR Apache-2.0** (see `Cargo.toml`).
