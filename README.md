<div align="center">

<!-- Add your banner/logo here -->

# RTunes

A beautiful terminal music player built in Rust

[![Version](https://img.shields.io/badge/version-v0.1.1-blue?style=flat-square)](https://github.com/TheCoder1232/RTunes/releases)
[![License](https://img.shields.io/badge/license-MIT-green?style=flat-square)](LICENSE)
[![Built with Rust](https://img.shields.io/badge/built%20with-Rust-orange?style=flat-square&logo=rust)](https://www.rust-lang.org/)
[![Build Status](https://img.shields.io/github/actions/workflow/status/TheCoder1232/RTunes/build.yml?style=flat-square)](https://github.com/TheCoder1232/RTunes/actions/workflows/build.yml)
[![GitHub Issues](https://img.shields.io/github/issues/TheCoder1232/RTunes?style=flat-square)](https://github.com/TheCoder1232/RTunes/issues)

</div>

---

## 📸 Preview

> Screenshot or GIF of the visualizer goes here — replace once available.

---

## ✨ Features

- 🎨 **Beautiful Audio Visualizer** — real-time TUI visualizer with multiple modes, the centerpiece of RTunes
- 🎵 **Local File Playback** — play music from your local filesystem with full playback controls
- 📥 **YouTube Music Downloads** — download tracks directly into your library using yt-dlp
- 📚 **Library Management** — add and manage multiple library folders from within the app
- ⌨️ **Keyboard-driven UI** — fast, distraction-free terminal controls for everything

---

## 🖥️ Platform Support

| Platform | Architecture | Status |
|----------|-------------|--------|
| Linux (Arch) | x86\_64 | ✅ Supported |
| Windows | x86\_64 | ✅ Supported |
| Windows | aarch64 | ✅ Supported |
| macOS | aarch64 (Apple Silicon) | ✅ Supported |

---

## 📦 Installation

### Option 1 — Pre-built Binaries (Recommended)

Download the latest binary for your platform from the [Releases](https://github.com/TheCoder1232/RTunes/releases) page.

**Linux / macOS:**

```bash
chmod +x rtunes
sudo mv rtunes /usr/local/bin/
```

**Windows:** Download the `.exe` from releases and add it to your PATH.

### Option 2 — Build from Source

**Prerequisites:** Rust stable toolchain and cargo — install from [https://www.rust-lang.org/tools/install](https://www.rust-lang.org/tools/install)

```bash
# 1. Clone the repository
git clone https://github.com/TheCoder1232/RTunes.git
cd RTunes

# 2. Build in release mode
cargo build --release

# 3. Binary output:
#    Linux/macOS: ./target/release/rtunes
#    Windows:     ./target/release/rtunes.exe

# 4. (Optional) Install system-wide on Linux/macOS
sudo mv ./target/release/rtunes /usr/local/bin/
```

---

## 🚀 Usage

```bash
rtunes
```

> Detailed keybindings and usage guide coming soon.

---

## 🗺️ Roadmap

RTunes is early in development at v0.1.1 — there is a lot more planned.

- [ ] Playlist support
- [ ] Equalizer settings
- [ ] Last.fm scrobbling
- [ ] Config file (~/.config/rtunes/config.toml)
- [ ] Streaming support

Have an idea? [Open a feature request →](https://github.com/TheCoder1232/RTunes/issues/new?template=feature_request.md)

---

## 🤝 Contributing

Contributions are welcome at all skill levels — whether it is a typo fix or a brand-new feature. See [CONTRIBUTING.md](CONTRIBUTING.md) for guidelines on how to get started.

---

## 📄 License

RTunes is licensed under the [MIT License](LICENSE).

© 2026 [TheCoder1232](https://github.com/TheCoder1232)

---

<div align="center">
<sub>Built with ❤️ and Rust</sub>
</div>
