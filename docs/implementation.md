# RTunes — Implementation Plan

**Language**: Rust 1.95+  
**Total phases**: 10  
**Estimated timeline**: ~12 weeks

Each phase builds on the previous and produces a runnable/testable artifact by its end.

---

## Phase 1 — Project Scaffolding & Core Infrastructure

**Goal**: Working Cargo project with all dependencies wired, logging active, and cross-platform config paths resolved. Nothing plays music yet — but the bones are solid.

### Tasks
- Initialize `cargo new rtunes --bin`
- Add all dependencies to `Cargo.toml` (ratatui, crossterm, rodio, rustfft, lofty, crossbeam-queue, crossbeam-channel, clap, serde, serde_yaml, dirs, shellexpand, dunce, url, sha2, hex, tracing, tracing-appender, tracing-subscriber, anyhow, thiserror)
- Set up `[profile.release]` with `strip = true`, `opt-level = 3`, `lto = true`
- Create directory skeleton: `src/{app,tui,audio,visualizer,fetcher,library,config}/`, `tests/{unit,integration}/`, `assets/`, `deps/`, `docs/`
- Implement `src/error.rs` — `RtunesError` enum via `thiserror` (Config, Audio, Visualizer, Fetcher, Io variants)
- Implement `src/utils.rs` — binary resolver (PATH → exe-adjacent `deps/`), `shellexpand::tilde` wrapper, `dunce::canonicalize` wrapper
- Implement `src/config/mod.rs` (stub) — cross-platform config path via `dirs::config_dir()`; create config dir if absent
- Initialize `tracing-subscriber` + `tracing-appender::rolling::daily` for file logging; log goes to platform-appropriate path
- Install panic hook in `main.rs` (`install_panic_hook()`) **before** any TUI setup — restores raw mode and alternate screen on crash
- Smoke-test: `cargo run` prints "RTunes starting…" and exits cleanly

### Deliverable
`cargo build --release` succeeds; logging writes to the correct platform path; `cargo test` passes with zero tests (placeholder).

---

## Phase 2 — Configuration, Themes & Data Models

**Goal**: Config file is fully loaded/saved; all data structures are defined; 5 built-in themes are hardcoded.

### Tasks
- Implement `src/config/mod.rs` — load `config.yaml` (serde_yaml), write defaults if missing, honor `RTUNES_CONFIG_PATH` env var
- Write `assets/default_config.yaml` with documented defaults (library_paths, download_dir, fps, default_visualizer, theme, fetcher, audio sections)
- Implement `src/config/theme.rs` — `Theme`, `VizTheme`, `BarStyle` structs; hardcode all 5 built-in themes (Synthwave, Dracula, Nord, Tokyo Night, Monochrome) with exact color values from spec
- Define all data models in `src/app/state.rs`:
  - `Track` (id via SHA256 of filepath, filepath, title, artist, album, duration_secs)
  - `PlayerState` (is_playing, volume, muted, current_index, position_secs, duration_secs, seek_to, shuffle, RepeatMode)
  - `InputMode` enum (Normal, Search, DownloadUrl, AddLibraryPath, LibraryManager)
  - `AppState` (library, filtered_indices, search_query, input_mode, input_buffer, player, visualizer_mode, is_fullscreen, neon_enabled, selected_track, message, download_progress, show_help, show_library_manager, library_folders, selected_folder, is_rescanning, quit)
  - `LibraryFolder` (path, track_count, last_scanned)
  - `VisualizerMode` enum (Spectrum, MirrorSpectrum, Spectrogram, Oscilloscope, Vectorscope, Supernova, PulseRings, BandMeter, Particles)
- Wrap `AppState` in `Arc<Mutex<AppState>>` in `src/app/mod.rs`; expose constructor `AppState::new(config)` with sensible defaults
- CLI skeleton in `src/cli.rs` using `clap derive`: subcommands `tui`, `fetch`, `scan`, `library`; global flags `--config`, `--log-level`; `tui` flags `--theme`, `--fps`, `--fullscreen`
- Wire `main.rs` to parse CLI and dispatch to stub handlers (print "not yet implemented" and exit)
- Unit tests: config round-trip (load → mutate → save → reload), theme lookup by name, `Track` SHA256 ID stability

### Deliverable
`rtunes tui` prints "TUI not yet implemented"; `rtunes --help` shows all subcommands with flags; config is created on first run; all structs compile cleanly.

---

## Phase 3 — Library Scanner & CLI Library Management

**Goal**: The library scanner finds audio files, extracts metadata, deduplicates paths, and the four `library` CLI subcommands work end-to-end.

### Tasks
- Implement `src/library/scanner.rs`:
  - `scan_paths(paths: &[PathBuf]) -> Vec<Track>` — recursive walk, filters by extension (mp3, flac, wav, m4a, opus, ogg, aac)
  - Deduplicate via `dunce::canonicalize` (handles Windows case/UNC differences and overlapping folders)
  - Extract metadata with `lofty` — title falls back to filename stem; duration fallback to 0
  - Generate `Track.id` as `hex(sha256(canonical_path_bytes))`
  - Background variant: `scan_async(paths, tx: Sender<ScanEvent>)` — fires `FolderStarted`, `Progress(scanned, total)`, `Done(Vec<Track>)`; guarded by `AtomicBool` single-flight flag
- Implement `rtunes scan` CLI command — synchronous scan, prints summary ("Scanned N folders. Found X tracks.")
- Implement `rtunes library add <PATH>` — validate path exists and is a directory (shellexpand then fs check), append to `library_paths` in config, save, run scan, print result
- Implement `rtunes library remove <PATH>` — match by `dunce::canonicalize`, remove from config, save, re-scan, print result
- Implement `rtunes library list` — print all paths; count tracks per folder
- Unit tests: scan a temp directory of stub files, verify deduplication, verify metadata fallbacks, verify `ScanEvent` sequence, verify single-flight guard prevents double-scan

### Deliverable
`rtunes library add ~/Music && rtunes scan` correctly indexes audio files and prints a track count. All `library` subcommands work from the terminal.

---

## Phase 4 — Audio Engine

**Goal**: Audio plays from a file path, play/pause/next/prev/seek/volume work, and PCM samples are captured for future visualizer use.

### Tasks
- Implement `src/audio/tap_source.rs` — `TapSource<S: Source>`:
  - Wraps any `rodio::Source`, intercepts every sample
  - Aggregates mono→stereo duplication or passes stereo pairs through as `(L, R)`
  - Pushes each `(f32, f32)` pair into `crossbeam_queue::ArrayQueue<StereoSample>` via `force_push` (drops oldest, never blocks)
  - Maintains `AtomicU64` sample counter for position derivation
- Implement `src/audio/player.rs` — `AudioPlayer`:
  - Initialize `rodio::OutputStream` + `OutputStreamHandle`; on failure log and enter **silent mode** (flag in AppState)
  - `load_track(path)` — open file, create decoder, wrap in `TapSource`, push to `Sink`
  - Seek strategy: try `Source::try_seek(Duration)` first; fallback to stop/rebuild/`skip_duration`/re-wrap
  - Expose `play`, `pause`, `resume`, `stop`, `seek_to`, `set_volume`, `mute`, `position_secs`
  - Audio thread loop: check `AppState.player.seek_to`, check skip/next/prev signals, report `position_secs` back via `AtomicU64`
- Define `AudioBackend` trait + `RodioBackend` (production) + `SilentBackend` (tests/headless)
- Handle auto-advance: when track ends (sink empty), increment `current_index` respecting repeat/shuffle, load next track
- Shuffle: Fisher-Yates on `filtered_indices`; "Repeat One" reloads same track; "Repeat All" wraps index
- Unit tests: `SilentBackend` play/pause/seek/volume state transitions; verify sample counter arithmetic; verify mute preserves prior volume

### Deliverable
Headless test: `cargo test audio` — tracks load, positions advance, seek teleports the counter, volume clamps to `[0.0, 1.0]`, silent mode activates when no device.

---

## Phase 5 — TUI Foundation & Render Loop

**Goal**: A fully interactive terminal UI with library list, now-playing bar, controls, hint bar, keyboard navigation, and search — no visualizer yet (blank background).

### Tasks
- Implement `src/tui/mod.rs` — terminal lifecycle: `enter_alternate_screen`, `enable_raw_mode`, RAII `TerminalGuard` that restores on `Drop` (covers normal exit; panic hook covers panics)
- Implement render loop in `src/tui/ui.rs`:
  - Frame budget: `frame_duration = 1000ms / fps`; sleep remainder; skip frame if over-budget
  - Layout (normal mode): 1-row now-playing bar → open visualizer space (blank for now) → 8-row library overlay → 2-row controls + progress bar → 1-row hint bar
  - Layout (fullscreen mode): visualizer fills everything; thin progress bar at bottom; hint bar hidden
  - `render_now_playing` — track title + artist in `theme.primary`; visualizer name + mode indicator top-right
  - `render_library_overlay` — scrollable list; selected row highlighted with `theme.primary`; dim background (`theme.surface`); no hard borders
  - `render_controls` — progress bar as gradient fill matching `viz.gradient`; position/duration; volume + shuffle/repeat indicators
  - `render_hint_bar` — context-sensitive text that adapts to `InputMode`
  - Toast rendering: `AppState.message` shown in bottom-right corner, auto-dismissed after 5s
  - Help overlay (`show_help`) — centered semi-transparent popup with all keybindings grouped by category
  - Library Manager overlay (`show_library_manager`) — folder list with track counts, action hints
- Implement `src/tui/events.rs` — keyboard dispatch for all keybindings listed in spec:
  - Playback: Space, n, p, →/←/l/h (5s), Shift+→/←/L/H (30s), +/=, -, m, s, r
  - Navigation: ↑/↓/j/k, Enter, /
  - Search mode: typing → update `input_buffer`; Enter → activate filter; Esc → cancel
  - Download prompt: typing URL; Enter → enqueue; Esc → cancel
  - Visualizer: v/V cycle, 1–9/0 direct jump, t (theme), g (glow toggle), f (fullscreen)
  - Library Manager: a (add folder prompt), x/Delete (remove), R (rescan), Esc (close)
  - General: ? (help), Esc (close overlay or quit), q (quit)
  - `crossterm::event::Event::Resize` → recalculate layout, adjust bin count hint in AppState
- Live search filtering: on every `input_buffer` change in Search mode, recompute `filtered_indices` (case-insensitive substring match on `title + artist + album`)
- Wire `rtunes tui` CLI dispatch to launch TUI, spawn audio thread, enter render loop
- Manual smoke test checklist: navigation, search, theme cycling, help overlay, resize

### Deliverable
`rtunes tui` launches a working interactive TUI. Library scrolls, search filters, all overlays open/close, controls display correctly — just no music or visualizer yet.

---

## Phase 6 — FFT Pipeline & Smoothing Engine

**Goal**: PCM samples from `TapSource` flow through to a dedicated FFT thread; the universal smoothing module is complete; `VisualizerData` is produced at ~30 Hz and delivered to the render loop.

### Tasks
- Implement the `ArrayQueue<StereoSample>` bridge (audio → FFT thread): size from config (`ring_buffer_size`), `force_push` from `TapSource`
- Implement `src/visualizer/fft.rs`:
  - Read `ring_buffer_size / fft_hop_size` stereo samples from the queue per tick
  - Downmix to mono for FFT (`(L + R) * 0.5`); preserve raw stereo buffer for `pcm_stereo`
  - Apply Hann window (`apply_hann_window`) on the mono buffer
  - Run `rustfft` forward FFT (size = `fft_window_size`); take magnitudes of first half
  - Convert to dB: `20 * log10(mag.max(EPSILON))`
  - Map onto N log-spaced bins (20 Hz → 20 kHz, N adaptive to terminal width hint)
  - Compute `bass_energy`, `mid_energy`, `high_energy` as band averages; `loudness` as RMS
- Implement `src/visualizer/smoothing.rs` — the universal smoothing module:
  - `apply_spectral_smoothing(bins)` — 3-tap weighted average `[0.25, 0.5, 0.25]`
  - `asymmetric_ema(prev, new, attack, release)` — per-bin EMA with configurable ATTACK/RELEASE constants
  - `peak_hold_drift(peak, smoothed, decay=0.96)` — floating peak markers
  - `OneEuroFilter` struct — adaptive low-pass filter (~30 lines, no deps); used for slow camera/parameter smoothing
  - `SpectralFluxBeatDetector` — flux → adaptive threshold → `beat: bool` for exactly one frame; `beat_intensity` decaying from 1.0 at `0.92/frame`; `bpm_estimate` via inter-onset intervals
  - `sub_frame_t(now, viz_timestamp, fft_period) -> f32` — `[0.0, 1.0]` interpolation parameter
- Assemble `VisualizerData` struct (all fields from spec): `bins_raw`, `bins_smoothed`, `bins_peak`, `bins_prev`, `pcm_mono`, `pcm_stereo`, band energies, beat fields, `spectrogram_rows` VecDeque, `timestamp`, `fft_period`
- Send via `crossbeam_channel::bounded(2)` (FFT thread → render loop); render loop uses `try_recv` and discards old frames
- Update render loop to call `sub_frame_t` each frame and pass `t: f32` into visualizer render functions
- Unit tests: sine wave at 440 Hz → verify peak bin is in the correct log-frequency bucket; beat detector fires on a synthetic impulse; EMA attack faster than release; One-Euro Filter reduces jitter on step input

### Deliverable
`cargo test visualizer` passes. In the running TUI, the FFT thread starts (log message), consumes samples, and pushes `VisualizerData` — render loop receives it (visible in DEBUG logs). Still no visual output yet.

---

## Phase 7 — Core Visualizers (Spectrum, Oscilloscope, Supernova)

**Goal**: Three polished, fully reactive visualizers are live. The visualizer renders as a full-screen background layer behind all UI overlays.

### Tasks
- Define `trait Visualizer { fn render(&mut self, frame: &mut Frame, area: Rect, data: &VisualizerData, t: f32, theme: &Theme); }` in `src/visualizer/mod.rs`
- Refactor render loop to call `render_visualizer(frame, full_area, ...)` **first** (background), then overlay all UI panels on top — matching the integrated rendering spec exactly
- Implement shared canvas helpers:
  - Braille canvas wrapper (Marker::Braille, coordinate transforms)
  - **Phosphor buffer** — persistent `Vec<Vec<f32>>` sized to canvas, multiply by `0.85` each frame, OR-paint new shape; draw from buffer
  - **Glow halo** — render shape twice: full brightness + 40% brightness offset ±1 dot in desaturated companion color (when `viz.glow` is true)
  - **Gradient color interpolation** — map a `[0.0, 1.0]` value onto `viz.gradient` color stops
  - Catmull-Rom spline helper for smooth curves
- Implement `src/visualizer/spectrum.rs` — Spectrum:
  - Adaptive bin count (32 / 48 / 64 based on terminal width)
  - Bar height = `lerp(bins_prev[i], bins_smoothed[i], t) * area.height`
  - Color per bar via gradient by height; bar style from `theme.viz.bar_style`
  - Peak dot at `bins_peak[i]` drifting down
  - Fullscreen mirror mode (reflected bars downward at 50% opacity)
  - ATTACK = 0.6, RELEASE = 0.15
- Implement `src/visualizer/oscilloscope.rs` — Oscilloscope:
  - Plot `pcm_mono` left-to-right on Braille canvas
  - Zero-crossing trigger: find first rising zero-crossing, start from there (prevents drift)
  - Catmull-Rom spline interpolation when sample count < canvas width
  - Phosphor compositing (decay 0.85); `tanh`-style soft amplitude clip at 80% height
  - Glow halo when enabled
- Implement `src/visualizer/supernova.rs` — Supernova:
  - 32 radial spokes at `360° / 32` intervals
  - Spoke length = `lerp(bins_prev[i], bins_smoothed[i], t) * R_max`
  - Inner radius pulses with `bass_energy`
  - Rotation: `0.3°/frame + mid_energy * 1.5°/frame`, fed through One-Euro Filter
  - Colors cycle through `viz.gradient` around the circle; core pulses in `particle_colors[0]`
- Visualizer cycling: `v` / `V` wrap through all 9 modes; `1`–`9` jump directly (order matches cycle)
- Unit/integration: render each visualizer into a fixed-size test frame with zeroed `VisualizerData` and confirm no panic; render with a beat frame and confirm beat_intensity drives size changes

### Deliverable
`rtunes tui` launches with a live, reactive Spectrum visualizer visible behind the UI. Switching to Oscilloscope and Supernova with `v`/`V` works. The UI panels are clearly overlaid on the visualizer background.

---

## Phase 8 — Full Visualizer Suite & Theme System

**Goal**: Nine music-tight visualizers are implemented (ambient screen-saver modes removed). Themes, glow toggle, and fullscreen mode are complete.

### Tasks
- Implement `src/visualizer/particles.rs` — Particles:
  - 150–400 particles (adaptive to terminal area); object pool — no allocations in hot path
  - Beat → 30-particle burst from center scaled by `beat_intensity`
  - Bass → outward force + radius increase; mids → Perlin-noise turbulence; highs → hue shift
  - Sub-stepped Euler physics (2 substeps/frame): center gravity `0.02 * dist`, damping `0.97/frame`
  - Lifecycle `[0,1]` decaying at `0.005/frame`; opacity = life; render as Braille dots
  - Temporal supersampling: prior frame at 60% brightness underneath
- Implement `src/visualizer/mirror_spectrum.rs` — MirrorSpectrum:
  - Same per-bin pipeline as Spectrum; bars grow up and down from a horizontal centerline
  - Bass third of bins brighten on `beat` / `beat_intensity`
- Implement `src/visualizer/pulse_rings.rs` — PulseRings:
  - Expanding ring only on beat frames (gated by `loudness`); hue from bass/mid/high balance; ~1.2s fade
- Implement `src/visualizer/band_meter.rs` — BandMeter:
  - Three vertical VU columns for `bass_energy` / `mid_energy` / `high_energy` with asymmetric EMA and peak-hold; beat flash on dominant band
- Implement `src/visualizer/vectorscope.rs` — Vectorscope:
  - Source: `pcm_stereo` (only stereo visualizer)
  - Standard rotation: `x = (L-R)*scale`, `y = (L+R)*scale`; centered
  - Phosphor buffer (decay 0.88); beam in `wave_color`, decay tints to `wave_trail`
  - Loudness modulates beam intensity; on beat, 2-pixel pen width
  - Catmull-Rom spline between sample points
- Implement `src/visualizer/spectrogram.rs` — Spectrogram:
  - Push new EMA-smoothed bins onto front of `spectrogram_rows` VecDeque each FFT frame
  - Render rows top-to-bottom; color = gradient interpolated by magnitude; below threshold = background
  - Vertical lerp during sub-frame: top row partial line colored `lerp(prev_top, current_top, t)`
  - Optional Gaussian blur (3×3 separable kernel, σ ≈ 0.8) for soft waterfall look
  - 3 display modes (standard / inverted / mirrored), cyclable with `m` while active
- Complete theme system:
  - `t` key cycles through built-in themes; wraps around
  - `g` key toggles `neon_enabled` (overrides `theme.viz.glow`); shows toast "Neon ON" / "Neon OFF"
  - Theme change takes effect on next render frame (no restart needed)
- Implement fullscreen mode toggle (`f` key):
  - Visualizer fills entire terminal; track name/artist subtle overlay auto-hides after 3s, reappears on track change
  - Thin 1-row progress line at bottom edge; hint bar hidden
  - Panels fade out/in over 3–4 frames on toggle (progressive text dimming)

### Deliverable
All 9 visualizers cycle correctly with `v`/`V`/`1`–`9`. Theme switching, glow toggle, and fullscreen mode work. Visualizers are biased toward tight audio coupling (spectrum, mirror spectrum, spectrogram, oscilloscope, vectorscope, supernova, pulse rings, band meter, particles).

---

## Phase 9 — Fetcher, TUI-First Downloads & Library Manager

**Goal**: yt-dlp downloads work both from the CLI and from within the TUI. The Library Manager overlay is fully functional. All TUI-first operations are complete.

### Tasks
- Implement `src/fetcher/downloader.rs`:
  - `pub trait Fetcher: Send + Sync { fn fetch(&self, url: &Url, opts: &FetchOpts, tx: Sender<FetchEvent>) -> Result<()>; }`
  - `YtDlpFetcher` — resolve yt-dlp + ffmpeg binaries (PATH → exe-adjacent `deps/`); spawn child via `std::process::Command` with `--newline --audio-format <fmt> --ffmpeg-location <path> --output "%(title)s.%(ext)s"`; `-x` for audio extraction
  - `BufReader` on child stdout; parse `[download]  XX.X%` regex → `FetchEvent::Progress(f32)`; parse stage lines ("Downloading…", "Extracting audio…", "Converting…") → `FetchEvent::Stage(String)`; parse `[ExtractAudio] Destination: …` → `FetchEvent::Done(PathBuf)`; non-zero exit → `FetchEvent::Failed(String)`
  - URL validation: `validate_url()` — scheme must be http/https; reject `;` and `|` characters
  - Bounded download queue: max `fetcher.max_concurrent` (default 3) simultaneous downloads; each on its own `std::thread`; pending URLs held in a bounded queue
  - On `Done`: trigger auto-reindex via scanner (reuse scanner worker thread); show toast "New track found: [title]"
  - `MockFetcher` — writes a stub `.mp3` file, emits scripted progress events for tests
- Wire fetcher into CLI: `rtunes fetch <URL> [--format] [--output]` — validates URL, spawns `YtDlpFetcher`, streams `FetchEvent` to stderr progress display, exits with code 0/1
- Wire fetcher into TUI:
  - `d` key opens URL input prompt; `Enter` enqueues URL; `Esc` cancels
  - Render loop drains `Sender<FetchEvent>` each tick; updates `AppState.download_progress` and toast text
  - Progress bar shown in controls area during active download
  - Error surface: yt-dlp not found → toast "yt-dlp not found. Place it in the `deps/` folder next to rtunes.exe, or install it to PATH."
- Complete Library Manager overlay (`Ctrl+l`):
  - Displays all `library_folders` with track counts; total count at bottom
  - `↑`/`↓`/`j`/`k` navigation; selected folder highlighted
  - `a` → opens `AddLibraryPath` input mode (validates path exists and is a directory before adding)
  - `x` / `Delete` → removes selected folder; shows confirmation toast; auto-reindexes; on last folder removed: toast "All library folders removed. Press 'a' to add a new one."
  - `R` (Shift+r) → triggers rescan; single-flight guard (second press shows "Rescan already in progress"); progress toast "Rescanning… Found X tracks."; on completion "Rescan complete. Found X tracks."
  - `Esc` closes overlay
  - `a` key outside Library Manager → opens `AddLibraryPath` input prompt directly
  - Rescan during playback: runs in background thread, does NOT interrupt audio; `current_index` repointed to same track in new library
- Handle all remaining edge cases: empty library placeholder, corrupted file skip-with-warning, terminal resize during visualizer, long track `HH:MM:SS` formatting

### Deliverable
`rtunes fetch <URL>` downloads a track from the terminal. Inside the TUI, `d` + URL + Enter downloads it. Library Manager opens, adds/removes folders, rescans. All edge cases in spec are handled.

---

## Phase 10 — Testing, Polish & Distribution

**Goal**: Test coverage meets targets, performance requirements are verified, release binary is built, and the project is documented.

### Tasks
- **Unit tests** (embedded `#[cfg(test)]` modules, >70% coverage target):
  - FFT binning with synthetic 440 Hz and 880 Hz sine waves — verify peak bin correct
  - Config round-trip with invalid YAML (parse failure returns descriptive error, not panic)
  - `Track` SHA256 ID stability across OS path styles
  - `asymmetric_ema` attack/release directionality
  - `SpectralFluxBeatDetector` fires on impulse, respects 250ms minimum interval
  - `OneEuroFilter` reduces jitter on noisy step input
  - `validate_url` rejects non-http(s) and suspicious characters; accepts valid YouTube URLs with `&`
  - `safe_path` rejects traversal attempts
- **Integration tests** (`tests/integration/`):
  - `MockFetcher` → verify stub file appears in library, `FetchEvent::Done` fires, auto-reindex triggers
  - Simulate keyboard events through event handler → verify `AppState` mutations (play/pause, seek, volume, search filter, theme cycle)
  - Load 1000-track library via temp directory → measure scan time (regression guard: must complete in < 2s)
  - `SilentBackend` + headless TUI render: one full render frame produces no panic; resize event recalculates layout correctly
- **Manual testing checklist**:
  - [ ] All 10 visualizers render correctly in Windows Terminal, Alacritty, iTerm2, Kitty
  - [ ] Terminal resize during playback: no panic, layout recalculates
  - [ ] Play 10+ tracks in sequence: no memory growth
  - [ ] Download invalid URL: graceful toast, no crash
  - [ ] Corrupted audio file: skipped with warning, next track plays
  - [ ] No audio device (headless): silent mode launches TUI, controls grayed, toast shown
  - [ ] `rtunes --help` and all subcommand `--help` output is accurate
- **Performance profiling**:
  - `cargo flamegraph -- tui` — identify hot paths in render loop and FFT thread
  - Verify: startup < 300ms cold, memory < 200MB with visualizers active, CPU < 8% on modern hardware
  - Minimize allocations in render loop (pre-allocate gradient Vec, reuse canvas buffers)
  - Use `try_recv` in render loop to drop stale FFT frames; never block render on FFT
- **Quality checks**:
  - Audit all `unwrap()` calls — replace with `?`, `unwrap_or_default()`, or documented `expect()` where invariant is guaranteed
  - All public API items have `///` doc comments
  - Error messages are actionable (tell user what to do, not just what failed)
  - No busy-wait loops (all waits are `sleep`-based or blocking channel receives)
  - Memory profile shows no leaks over 1-hour runtime (use `heaptrack` or Valgrind on Linux)
- **Release build & distribution packaging**:
  - `cargo build --release` with strip + LTO
  - Assemble distribution archive: `rtunes-v0.1.0-<platform>/rtunes[.exe]` + `deps/` (yt-dlp, ffmpeg, ffprobe) + `README.md`
  - Write `README.md`: installation, quick-start, keybinding reference, config reference, deps folder setup
  - `.gitignore`: exclude `deps/`, `target/`, log files, `config.yaml`

### Deliverable
`cargo test` passes. Release binary runs correctly on all target platforms. Distribution zip is ready. README covers installation and first use.

---

## Phase Summary

| # | Phase | Key Output |
|---|---|---|
| 1 | Scaffolding & Infrastructure | Compilable project, logging, panic hook |
| 2 | Config, Themes & Data Models | Config loaded, 5 themes, all structs defined |
| 3 | Library Scanner & CLI | `rtunes scan` + `library` commands work |
| 4 | Audio Engine | Music plays headlessly, TapSource captures PCM |
| 5 | TUI Foundation | Interactive UI, navigation, search, overlays |
| 6 | FFT Pipeline & Smoothing | VisualizerData produced at 30 Hz, all smoothing primitives |
| 7 | Core Visualizers | Spectrum, Oscilloscope, Supernova live behind UI |
| 8 | Full Visualizer Suite | All 10 visualizers, themes, glow, fullscreen |
| 9 | Fetcher & Library Manager | Downloads work in TUI + CLI, Library Manager overlay |
| 10 | Testing, Polish & Distribution | Tests pass, perf verified, release binary packaged |
