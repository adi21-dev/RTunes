# Contributing to RTunes

Thanks for your interest in contributing! Here's how to get started.

## Prerequisites

- **Rust 1.94+** (stable toolchain)
- **Linux only**: ALSA dev headers (`sudo apt install libasound2-dev` on Debian/Ubuntu)
- **Optional**: `yt-dlp` and `ffmpeg` for download features

## Setup

```bash
git clone https://github.com/TheCoder1232/rtunes.git
cd rtunes
cargo build
cargo test
```

## Development workflow

1. Fork the repo and create a branch from `dev`:
   ```bash
   git checkout dev
   git checkout -b feature/my-change
   ```
2. Make your changes.
3. Run checks before committing:
   ```bash
   cargo fmt --all
   cargo clippy -- -D warnings
   cargo test
   ```
4. Open a PR targeting the `dev` branch.

## Commit messages

Use [Conventional Commits](https://www.conventionalcommits.org/) style:

- `feat: add waveform visualizer`
- `fix: handle missing audio device gracefully`
- `refactor: simplify FFT pipeline`
- `chore: update dependencies`

## Code style

- Follow `rustfmt` defaults (enforced by CI).
- No warnings under `clippy -D warnings`.
- Keep unsafe code to zero — if you think you need it, open an issue first.

## Adding a visualizer

Each visualizer lives in `src/visualizer/renderers/`. See any existing renderer (e.g. `spectrum.rs`) for the trait to implement. Register it in `renderers/mod.rs`.

## Changelog

If your change is user-facing (new feature, bug fix, breaking change), add an entry under `[Unreleased]` in `CHANGELOG.md`.

## License

By contributing, you agree that your contributions will be licensed under the [MIT License](LICENSE).
