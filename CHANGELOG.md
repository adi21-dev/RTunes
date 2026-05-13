# Changelog

All notable changes to this project will be documented in this file.

The format is based on [Keep a Changelog](https://keepachangelog.com/en/1.1.0/),
and this project adheres to [Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

<!-- Add entries here as you work on dev. When releasing, rename this section to the version + date. -->

## [0.2.0] - 2026-05-13

### Added
- Lazy auto-download of yt-dlp and ffmpeg on first use; binaries saved to `deps/` beside the executable and reused automatically
- `deps.rs` module with platform-aware asset resolution for yt-dlp and ffmpeg
- `ureq` HTTP client for dependency downloads

### Changed
- Release archives no longer bundle yt-dlp and ffmpeg; users get them automatically on first run or can install manually
- README updated with manual install table (winget / brew / apt) for yt-dlp and ffmpeg
- `Cross.toml` added to fix aarch64 Linux cross-compilation (installs `libwayland-dev:arm64` and `libasound2-dev:arm64` in the cross sysroot)

### Removed
- macOS x86\_64 (`macos-13`) release target dropped; only Apple Silicon (`aarch64-apple-darwin`) is now shipped

## [0.1.0] - 2026-05-12

### Added
- Terminal music player with Ratatui UI
- 10 audio-reactive visualizers (spectrum, oscilloscope, particles, phosphor, pulse rings, band meter, vectorscope, spectrogram, supernova, canvas)
- yt-dlp integration for downloading tracks
- Multi-platform release builds (Windows, Linux, macOS — x86_64 and aarch64)
- YAML-based config with theme support
- Library scanner with SHA2-based deduplication
- FFT-based audio analysis with smoothing

[Unreleased]: https://github.com/TheCoder1232/rtunes/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/TheCoder1232/rtunes/compare/v0.1.0...v0.2.0
[0.1.0]: https://github.com/TheCoder1232/rtunes/releases/tag/v0.1.0
