# Project Specification

## Overview
**Name**: RTunes  
**Type**: CLI Tool / TUI Application  
**Language**: Rust 1.95+  
**Complexity**: Medium-High (Multithreading, real-time audio processing, subprocess management, smooth TUI rendering)

## Purpose
RTunes is a visually stunning, high-performance terminal-based music player. It manages and plays a local library of audio files, integrates yt-dlp and FFmpeg to fetch and convert media directly from URLs, and renders beautiful, highly responsive audio visualizers (using Fast Fourier Transform) in the terminal.

## Design Philosophy — TUI-First

RTunes follows a **TUI-first** design principle: **every feature is accessible from within the TUI itself**. Regular users should never need to drop to a terminal to manage their library, download tracks, or change settings. The TUI provides overlays, prompts, and panels for all operations.

CLI commands (`rtunes fetch`, `rtunes scan`, `rtunes library`) exist as **power-user shortcuts** for scripting, automation, and headless workflows — but they are never _required_. Everything the CLI can do, the TUI can do too.

| Operation | TUI Access | CLI Equivalent |
|---|---|---|
| Download from URL | `d` key → URL prompt | `rtunes fetch <URL>` |
| Add library folder | `a` key → path prompt | `rtunes library add <PATH>` |
| Remove library folder | `Ctrl+l` → Library Manager → `x` | `rtunes library remove <PATH>` |
| View library folders | `Ctrl+l` → Library Manager | `rtunes library list` |
| Rescan library | `R` (Shift+r) | `rtunes scan` |
| Change theme | `t` key (cycle) | `--theme` flag |
| Change visualizer | `v` / `V` keys | `--fullscreen` flag |

## Target Users
Terminal enthusiasts, developers, and power users who want an aesthetic, keyboard-driven music player and screensaver for their desktop. Also regular users who prefer a self-contained graphical-style experience within the terminal.

---

## Technical Stack

### Core
- **Language**: Rust 1.95+
- **Build Tool**: Cargo
- **Package Manager**: Cargo / crates.io

### Dependencies
```toml
# TUI & Terminal
ratatui = "0.29"           # TUI rendering
crossterm = "0.28"         # Cross-platform terminal backend

# Audio
rodio = { version = "0.19", features = ["symphonia-all"] }  # Playback + decoding
rustfft = "6.2"            # Fast Fourier Transform

# Metadata (replaces symphonia for tag reading — more ergonomic)
lofty = "0.22"             # ID3/Vorbis/FLAC tag extraction

# Threading
crossbeam-queue = "0.3"    # Lock-free ring buffer for PCM samples
crossbeam-channel = "0.5"  # MPMC channels for fetcher/scanner progress events

# CLI
clap = { version = "4", features = ["derive"] }

# Config & Serialization
serde = { version = "1", features = ["derive"] }
serde_yaml = "0.9"
dirs = "5"                 # Cross-platform config/data paths
shellexpand = "3"          # Expand `~` and env vars in config paths
dunce = "1"                # Sane path canonicalization on Windows (no UNC \\?\ prefix)

# URL validation
url = "2"

# Track ID hashing
sha2 = "0.10"
hex = "0.4"

# Logging
tracing = "0.1"
tracing-appender = "0.2"
tracing-subscriber = { version = "0.3", features = ["env-filter"] }

# Error handling
anyhow = "1"
thiserror = "1"
```

---

## Architecture

### Directory Structure
```
project-root/
├── src/
│   ├── main.rs            # Entry point, wires everything together
│   ├── cli.rs             # CLI argument definitions (clap)
│   ├── error.rs           # Custom error types (thiserror)
│   ├── utils.rs           # Binary resolution (PATH + deps/ folder)
│   ├── app/               # Application state management
│   │   ├── mod.rs
│   │   └── state.rs       # Shared state (Arc<Mutex<AppState>>)
│   ├── tui/               # UI rendering and input handling
│   │   ├── mod.rs         # Terminal init/cleanup
│   │   ├── ui.rs          # Layout and rendering logic
│   │   └── events.rs      # Keyboard event handling
│   ├── audio/             # Audio playback thread
│   │   ├── mod.rs
│   │   ├── player.rs      # rodio Sink wrapper + controls
│   │   └── tap_source.rs  # TapSource<S> — PCM sample interception
│   ├── visualizer/        # FFT processing and visual engines
│   │   ├── mod.rs         # FFT thread management + VisualizerData
│   │   ├── fft.rs         # FFT computation (rustfft, Hann window)
│   │   ├── smoothing.rs   # Shared smoothing primitives (EMA, OneEuro, peak hold, beat detector)
│   │   ├── spectrum.rs    # Linear frequency bars
│   │   ├── oscilloscope.rs # Braille time-domain waveform
│   │   ├── supernova.rs   # Radial spectrum (polar bars)
│   │   ├── particles.rs   # Audio-reactive particle system
│   │   ├── aurora.rs      # Layered flowing sine waves
│   │   ├── starfield.rs   # 3D perspective star fly-through
│   │   ├── vectorscope.rs # Lissajous stereo (L×R) phase plot
│   │   ├── spectrogram.rs # Scrolling waterfall time-frequency view
│   │   ├── tunnel.rs      # Concentric perspective rings (warp tunnel)
│   │   └── matrix_rain.rs # Falling glyph columns (Matrix-style)
│   ├── fetcher/           # yt-dlp subprocess wrapper
│   │   ├── mod.rs
│   │   └── downloader.rs
│   ├── library/           # Music library scanner
│   │   ├── mod.rs
│   │   └── scanner.rs
│   └── config/            # Configuration management
│       ├── mod.rs         # Load/save config.yaml
│       └── theme.rs       # Theme structs + built-in palettes
├── tests/
│   ├── unit/
│   └── integration/
├── assets/
│   └── default_config.yaml
├── deps/                  # Bundled binaries (yt-dlp, ffmpeg) — not in git
├── docs/
├── .gitignore
├── Cargo.toml
└── README.md
```

### Module Breakdown
- **src/cli.rs**: Command-line argument parsing with clap
- **src/app/**: Central application state with thread-safe access
- **src/utils.rs**: Binary resolution (PATH first, then exe-adjacent `deps/` folder), path expansion via `shellexpand`, canonicalization via `dunce`
- **src/tui/**: Render loop (60 FPS via std::thread::sleep + crossterm poll), input handling, layout management
- **src/audio/**: Audio playback thread, PCM data extraction via TapSource (incl. stereo→mono downmix + sample-counter for position tracking)
- **src/visualizer/**: FFT computation thread (runs at ~30 Hz, independent of render FPS)
- **src/fetcher/**: yt-dlp subprocess execution on a dedicated worker thread with line-buffered stdout parsing for progress (no async runtime)
- **src/library/**: File system scanning on a background scanner thread, metadata extraction via lofty
- **src/config/**: YAML config and theme management

---

## Threading Architecture

### Thread Model
1. **Main Thread**: TUI render loop (30-60 FPS)
2. **Audio Thread**: Rodio playback (continuous, owned by `OutputStream`)
3. **FFT Thread**: Processes audio samples → frequency data (20-60 Hz)
4. **Fetcher Worker(s)**: One short-lived `std::thread` per active yt-dlp download (max 3, bounded by `fetcher.max_concurrent`)
5. **Scanner Worker**: One `std::thread` for library scans; rescans coalesce via a single-flight flag (`is_rescanning`)

> **No async runtime.** The original plan reserved tokio for yt-dlp subprocess management. Since RTunes has no network I/O of its own (yt-dlp does all networking) and only ever spawns 1–3 subprocesses, a plain `std::process::Command` + `BufReader` on a dedicated thread is simpler, lighter, and avoids pulling tokio into the dependency tree.

### Inter-Thread Communication
- **Audio → FFT**: `crossbeam_queue::ArrayQueue<StereoSample>` (where `StereoSample = (f32, f32)`) — lock-free, bounded ring buffer. `force_push()` drops the oldest pair if full (never blocks audio thread). **Stereo is preserved end-to-end**: the FFT thread downmixes to mono internally for the FFT, but the Vectorscope and stereo-aware visualizers consume the raw L/R pairs directly. Mono sources are duplicated to both channels at the source.
- **FFT → UI**: `crossbeam_channel::bounded(2)` — drop old frames if UI is slow, always show latest.
- **Fetcher → UI**: `crossbeam_channel::Sender<FetchEvent>` with variants `Progress(f32)`, `Stage(String)`, `Done(PathBuf)`, `Failed(String)`. Drained by the main thread once per render tick.
- **Scanner → UI**: `crossbeam_channel::Sender<ScanEvent>` with variants `FolderStarted(PathBuf)`, `Progress(usize, usize)`, `Done(Vec<Track>)`. Replaces `AppState.library` atomically when complete.
- **UI ↔ Audio**: `Arc<Mutex<AppState>>` for controls (play/pause/volume) and one-shot commands (`seek_to`, `skip_to_next`).

### PCM Sample Interception & Position Tracking
- **TapSource<S>**: A custom `rodio::Source` wrapper that intercepts every PCM sample before it reaches the audio output. The wrapper:
  1. Maintains an `AtomicU64` sample counter, used to derive `position_secs = counter / sample_rate / channels`.
  2. Aggregates incoming samples into stereo pairs (`(L, R)`). Mono sources fill both fields with the same value.
  3. Pushes each pair into the `ArrayQueue<StereoSample>` ring buffer for the FFT thread (`force_push` — zero-overhead, never blocks).
- The FFT thread downmixes to mono (`(L + R) * 0.5`) for spectral analysis but exposes the raw stereo buffer for the Vectorscope, which needs unsummed L vs R.
- The output device's actual sample rate (queried from `OutputStream::config()`) is captured at startup and passed into the FFT module so log-frequency bins map correctly regardless of 44.1k vs 48k vs 96k devices.

### Seek Strategy
rodio's `Sink` does not support arbitrary seeking on a playing source. Strategy:

1. **Preferred path**: call `Source::try_seek(Duration)` on the wrapped decoder. As of rodio 0.19, common formats (MP3, FLAC, WAV, Vorbis) support this for most files. If it returns `Ok`, just reset the sample counter to the new position.
2. **Fallback path** (decoder doesn't support `try_seek`, e.g. some Opus streams or corrupted files): stop the sink, rebuild the decoder from the file, call `skip_duration(Duration)`, re-wrap in `TapSource`, push to a new sink. The brief audio gap (~50–100ms) is acceptable for a UX that's already "seeking."
3. **State machine**: `PlayerState.seek_to: Option<f64>` is the request flag. Audio thread consumes it, attempts (1), falls back to (2), clears the flag, and updates `position_secs` atomically.

### Audio Device Failure Fallback
On `OutputStream::try_default()` failure (no audio device, WSL without WSLg, headless CI, ALSA misconfig, etc.):
- Log the error.
- Launch the TUI anyway in **silent mode**: visualizer renders an idle "no signal" animation, playback controls are disabled and grayed out, error toast displayed: `"No audio device available — TUI running in silent mode."`
- Library browse, search, theme switching, and downloads still work. This makes the app debuggable on any machine.

---

## Features & Commands

### Command 1: `tui` (Default)
**Purpose**: Launch the terminal user interface  
**Usage**: `rtunes tui [OPTIONS]`

**Flags**:
- `--theme <NAME>`: Color scheme (default: "dracula")
- `--fps <NUM>`: Target render FPS, 30-60 (default: 60)
- `--fullscreen`: Start in fullscreen visualizer mode

**Logic**:
1. Load config from `~/.config/rtunes/config.yaml` (Linux), `~/Library/Application Support/rtunes/config.yaml` (macOS), or `%APPDATA%\rtunes\config.yaml` (Windows)
2. Initialize crossterm alternate screen
3. Spawn audio playback thread with rodio
4. Spawn FFT processing thread
5. Enter render loop with crossterm event polling
6. Handle keyboard inputs:
   - **Playback**:
     - `Space`: Toggle play/pause
     - `n`: Next track
     - `p`: Previous track
     - `→`/`l`: Seek forward 5 seconds
     - `←`/`h`: Seek backward 5 seconds
     - `Shift+→`/`Shift+l`: Seek forward 30 seconds
     - `Shift+←`/`Shift+h`: Seek backward 30 seconds
     - `+`/`=`: Volume up (+5%)
     - `-`: Volume down (-5%)
     - `m`: Mute/unmute
     - `s`: Toggle shuffle
     - `r`: Toggle repeat (off → all → one)
   - **Navigation**:
     - `↑`/`k`: Scroll library up
     - `↓`/`j`: Scroll library down
     - `Enter`: Play selected track
     - `/`: Search/filter library — opens an inline search bar; case-insensitive substring match across `title + artist + album`; filters live as you type. `Enter` confirms and returns to Normal mode (filter remains active until cleared with `Esc` from Normal mode).
   - **Visualizer & Display**:
     - `v`: Cycle visualizers forward (Spectrum → Oscilloscope → Supernova → Particles → Aurora → Starfield → Vectorscope → Spectrogram → Tunnel → MatrixRain → wrap)
     - `V` (Shift+v): Cycle visualizers backward
     - `1`–`9`, `0`: Jump directly to visualizer N (1=Spectrum … 0=MatrixRain)
     - `t`: Cycle through themes
     - `g`: Toggle neon/glow effects (overrides theme default; shows toast "Neon ON" / "Neon OFF")
     - `f`: Toggle fullscreen visualizer mode (visualizer takes entire screen)
   - **Library Management (TUI-first — no CLI needed)**:
     - `d`: Download from URL (opens input prompt in TUI)
     - `a`: Add library folder (opens folder path input)
     - `Ctrl+l`: Open Library Manager overlay (view/add/remove folders, rescan)
     - `R` (Shift+r): Rescan library (reindex all configured folders)
   - **General**:
     - `?`: Toggle help overlay (shows all keybindings)
     - `Esc`: Close overlay/prompt (if active) / Quit (if not)
     - `q`: Quit
7. Render UI at target FPS (frame skipping if terminal too slow)

**Help System**:
- **Hint bar**: A compact 1-row bar always visible at the very bottom of the screen showing the most essential keys: `[Space] Play/Pause  [n/p] Next/Prev  [←/→] Seek  [?] Help  [q] Quit`
- **Help overlay** (`?`): A centered semi-transparent popup listing ALL keybindings grouped by category (Playback, Navigation, Library Management, Visualizer). Press `?` or `Esc` to dismiss.
- The hint bar text adapts to context:
  - Normal mode: `[Space] Play/Pause  [n/p] Next/Prev  [←/→] Seek  [?] Help  [q] Quit`
  - Search mode: `[Enter] Confirm  [Esc] Cancel`
  - Download prompt: `[Enter] Download  [Esc] Cancel`
  - Library Manager: `[a] Add  [x] Remove  [R] Rescan  [Esc] Close`

**Library Manager Overlay** (`Ctrl+l`):
A centered semi-transparent popup showing all configured library folders. This is the TUI-equivalent of `rtunes library list/add/remove` and `rtunes scan` — regular users never need to touch the CLI.

```
┌──────────── Library Manager ─────────────┐
│                                          │
│  📁 Library Folders                      │
│  ─────────────────────────────────────── │
│  ▸ D:\Music                   215 tracks │
│    D:\Downloads\Music          32 tracks │
│    E:\Lossless                  89 tracks │
│                                          │
│  Total: 336 tracks                       │
│                                          │
│  [a] Add folder  [x] Remove selected     │
│  [R] Rescan all  [Esc] Close             │
└──────────────────────────────────────────┘
```

- Navigate folders with `↑`/`↓`/`j`/`k`
- `a`: Opens path input prompt to add a new folder (validates it exists and is a directory)
- `x` or `Delete`: Remove the selected folder from library paths (with confirmation toast)
- `R` (Shift+r): Rescan all folders (shows progress toast: "Rescanning... Found X tracks.")
- `Esc`: Close overlay and return to normal mode
- After any add/remove, the library auto-reindexes and the track list updates immediately

**Files**: `src/tui/ui.rs`, `src/tui/events.rs`, `src/app/state.rs`

---

### Command 2: `fetch`
**Purpose**: Download audio from URL using yt-dlp, convert to audio format, and save to library  
**Usage**: `rtunes fetch <URL> [OPTIONS]`

**Flags**:
- `--format <FMT>`: Output format (mp3, flac, opus, m4a) (default: "mp3")
- `--output <DIR>`: Save location (default: first library path from config)

**Logic**:
1. Validate URL format (scheme must be http/https).
2. Resolve yt-dlp binary: PATH first → exe-adjacent `deps/` folder.
3. Resolve ffmpeg binary: PATH first → exe-adjacent `deps/` folder (pass via `--ffmpeg-location`).
4. Spawn yt-dlp on a dedicated `std::thread` via `std::process::Command` with `--newline` (forces yt-dlp to emit one progress line per update instead of `\r`-overwriting):
   ```
   yt-dlp -x --audio-format <format> --newline \
          --ffmpeg-location <path> \
          --output "<dir>/%(title)s.%(ext)s" <URL>
   ```
   - `-x` extracts audio only (even from video URLs).
   - `--audio-format mp3` ensures conversion via ffmpeg.
5. The fetcher thread wraps the child's stdout in a `BufReader` and parses each line for the `[download]  XX.X%` pattern.
6. Each parsed event is pushed through a `crossbeam_channel::Sender<FetchEvent>`:
   - `FetchEvent::Stage(String)` — yt-dlp lifecycle messages ("Downloading", "Extracting audio", "Converting").
   - `FetchEvent::Progress(f32)` — 0.0–1.0 percentage.
   - `FetchEvent::Done(PathBuf)` — final output path (parsed from the `[ExtractAudio] Destination: …` line).
   - `FetchEvent::Failed(String)` — non-zero exit code or parse failure.
7. The main thread drains the receiver once per render tick and updates `AppState.download_progress` + toast text.
8. On `Done`, the main thread triggers an auto-reindex of the library (reuses the scanner worker thread; UI shows "New track found: ...").
9. Handle errors: network issues, invalid URL, ffmpeg missing, yt-dlp missing — all surface as toasts in TUI or stderr in CLI.

**Concurrency**: Up to `fetcher.max_concurrent` (default 3) simultaneous downloads, each on its own thread. A bounded queue holds pending URLs.

**Note**: This works with any URL that yt-dlp supports — YouTube, SoundCloud, Bandcamp, direct video/audio URLs, etc. The video is downloaded and automatically converted to audio-only.

**Files**: `src/fetcher/downloader.rs`, `src/cli.rs`

---

### Command 3: `scan`
**Purpose**: Rebuild library index from all configured music directories  
**Usage**: `rtunes scan`

**Logic**:
1. Read `library_paths` list from config; expand `~` and env vars via `shellexpand`.
2. For each path, recursively find all audio files (mp3, flac, wav, m4a, opus, ogg, aac).
3. Deduplicate by canonical filepath via `dunce::canonicalize` (handles overlapping folders + Windows path-case differences).
4. Extract metadata (title, artist, album, duration) using `lofty`.
5. Build new `Vec<Track>` and atomically swap into `AppState.library`.
6. Print summary: "Scanned N folders. Found X tracks."

**Threading**:
- In TUI mode, scans run on a dedicated **scanner worker thread** so the UI never freezes. A `crossbeam_channel` reports `ScanEvent::Progress(scanned, total)` for live progress display.
- A single-flight guard (`is_rescanning: AtomicBool`) ensures concurrent rescan requests coalesce — the second press is a no-op with a "Rescan already in progress" toast.
- In CLI mode (`rtunes scan` from terminal), it runs synchronously on the main thread for predictable exit behavior.

**Files**: `src/library/scanner.rs`

---

### Command 4: `library`
**Purpose**: Manage library folders  
**Usage**: `rtunes library <SUBCOMMAND>`

**Subcommands**:
- `rtunes library add <PATH>` — Add a folder to the library paths. Auto-reindexes.
- `rtunes library remove <PATH>` — Remove a folder from library paths. Auto-reindexes (matches by `dunce::canonicalize` so case differences on Windows still resolve).
- `rtunes library list` — List all currently configured library folders.

**Logic**:
1. Load config
2. `add`: Validate path exists and is a directory, append to `library_paths`, save config, run scan
3. `remove`: Find and remove matching path from `library_paths`, save config, run scan
4. `list`: Print all paths with track count per folder

**Files**: `src/library/scanner.rs`, `src/config/mod.rs`, `src/cli.rs`

---

### Global Options
- `--config <PATH>`: Custom config file path
- `--log-level <LEVEL>`: Override logging (error/warn/info/debug/trace)

---

## Data Models

### Track
```rust
/// Represents an audio file in the library
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,          // SHA256 hash of filepath
    pub filepath: PathBuf,   // Absolute path
    pub title: String,       // From ID3 or filename
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_secs: u64,
}
```

### AppState
```rust
/// Central application state (shared via Arc<Mutex<>>)
pub enum InputMode {
    Normal,                                 // Regular keybindings
    Search,                                 // Typing search query
    DownloadUrl,                            // Typing URL to download
    AddLibraryPath,                         // Typing folder path to add
    LibraryManager,                         // Browsing library folders overlay
}

pub struct AppState {
    pub library: Vec<Track>,
    pub filtered_indices: Vec<usize>,       // Indices into library matching search
    pub search_query: Option<String>,       // Active search filter
    pub input_mode: InputMode,              // Current input mode
    pub input_buffer: String,               // Text being typed in prompt
    pub player: PlayerState,
    pub visualizer_mode: VisualizerMode,
    pub is_fullscreen: bool,
    pub neon_enabled: bool,                 // Runtime glow toggle (initialized from theme.viz.glow, toggled with 'g')
    pub selected_track: usize,              // Cursor position in library/filtered list
    pub message: Option<(String, Instant)>, // Toast notification (auto-dismiss)
    pub download_progress: Option<f32>,     // 0.0-1.0 during fetch
    pub show_help: bool,                    // Toggle help overlay
    pub show_library_manager: bool,         // Toggle library manager overlay
    pub library_folders: Vec<LibraryFolder>, // Configured folders with metadata
    pub selected_folder: usize,             // Cursor in library manager
    pub is_rescanning: bool,                // True while rescan is in progress
    pub quit: bool,
}

/// Represents a configured library folder with cached metadata
pub struct LibraryFolder {
    pub path: PathBuf,
    pub track_count: usize,                 // Number of tracks found in this folder
    pub last_scanned: Option<Instant>,      // When this folder was last indexed
}

pub struct PlayerState {
    pub is_playing: bool,
    pub volume: f32,              // 0.0 to 1.0
    pub muted: bool,              // Mute toggle (remembers volume)
    pub current_index: Option<usize>,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub seek_to: Option<f64>,     // Set by UI, consumed by audio thread
    pub shuffle: bool,
    pub repeat: RepeatMode,       // Off / All / One
}
```

### Theme
```rust
/// Complete theme definition — controls both UI and visualizer appearance
#[derive(Debug, Clone, Deserialize)]
pub struct Theme {
    pub name: String,

    // UI colors
    pub background: String,       // Hex: "#000000" — main background
    pub surface: String,          // Hex: "#1a1a2e" — panels, cards, borders
    pub primary: String,          // Hex: "#FF00FF" — active elements, highlights
    pub secondary: String,        // Hex: "#00FFFF" — secondary highlights
    pub text: String,             // Hex: "#FFFFFF" — primary text
    pub text_dim: String,         // Hex: "#888888" — secondary/inactive text
    pub accent: String,           // Hex: "#50fa7b" — success, active indicators

    // Visualizer-specific styling
    pub viz: VizTheme,
}

#[derive(Debug, Clone, Deserialize)]
pub struct VizTheme {
    pub gradient: Vec<String>,    // Color stops for visualizer gradients (bottom→top)
    pub glow: bool,               // Enable glow/bloom effect (dim halo around bright elements)
    pub bar_style: BarStyle,      // Solid, Rounded, Dots
    pub particle_colors: Vec<String>, // Palette for particle system
    pub wave_color: String,       // Primary waveform color (oscilloscope)
    pub wave_trail: String,       // Trailing/echo color (dim version of wave_color)
}

pub enum BarStyle {
    Solid,     // █ full block characters
    Rounded,   // ▓▒░ gradient blocks for softer look
    Dots,      // ⣿⡇ Braille characters for high-res bars
}
```

### VisualizerData
```rust
/// Per-frame data delivered from the FFT thread to the renderer.
/// All visualizers read from this; each picks the fields it needs.
pub struct VisualizerData {
    // --- Frequency-domain (FFT @ ~30 Hz) ---
    pub bins_raw: Vec<f32>,          // Log-magnitude bins, 0.0–1.0 (post-Hann + log-scale)
    pub bins_smoothed: Vec<f32>,     // Asymmetric-EMA smoothed (fast attack, slow release)
    pub bins_peak: Vec<f32>,         // Peak-hold values with slow drift (for peak markers)
    pub bins_prev: Vec<f32>,         // Previous frame's smoothed bins (for sub-frame interpolation)

    // --- Time-domain (raw PCM, updated every audio chunk) ---
    pub pcm_mono: Vec<f32>,          // Last 2048 mono samples for oscilloscope, aurora, tunnel
    pub pcm_stereo: Vec<(f32, f32)>, // Last 2048 stereo pairs for vectorscope

    // --- Frequency-band energies (precomputed for convenience) ---
    pub bass_energy: f32,            // 20–250 Hz, smoothed 0.0–1.0
    pub mid_energy: f32,             // 250 Hz–4 kHz, smoothed
    pub high_energy: f32,            // 4 kHz+, smoothed
    pub loudness: f32,               // Overall RMS, smoothed (for global brightness/saturation)

    // --- Beat detection (spectral flux onset detector) ---
    pub beat: bool,                  // True for exactly one frame on detected onset
    pub beat_intensity: f32,         // 0.0–1.0, decays after a beat (drives "kick" effects)
    pub bpm_estimate: Option<f32>,   // Inter-onset-interval-based estimate, None until stable

    // --- Spectrogram history (ring buffer of past FFT frames) ---
    pub spectrogram_rows: VecDeque<Vec<f32>>, // Last N frames (N ≈ terminal height in waterfall mode)

    // --- Timing for sub-frame interpolation ---
    pub timestamp: Instant,          // When this frame was produced
    pub fft_period: Duration,        // Expected interval to next FFT frame; renderer uses this for lerp
}
```

> **Sub-frame interpolation contract**: The render loop runs at 60 FPS but FFT only produces frames at ~30 Hz. Each visualizer's render function receives an additional `t: f32` in `[0.0, 1.0]` representing how far we are between `bins_prev` and `bins_smoothed`. Visualizers lerp between the two for silky 60 FPS movement even when the underlying analysis is slower.

---

## Configuration

### Config File (`config.yaml`)
```yaml
app:
  library_paths:                        # Multiple folders supported
    - "~/Music"                         # Default library folder
  download_dir: "~/Music"              # Where fetched tracks are saved
  fps: 60                               # 30-60 recommended
  default_visualizer: "spectrum"        # spectrum/oscilloscope/supernova/particles/aurora/starfield/vectorscope/spectrogram/tunnel/matrix_rain
  start_fullscreen: false
  log_level: "warn"

theme:
  active: "synthwave"
  # 5 built-in themes — each has a distinct visual personality
  # Users can also define custom themes under 'custom:'
  # Themes control BOTH the UI and visualizer appearance

fetcher:
  ytdlp_path: "auto"        # "auto" = PATH first, then exe-adjacent deps/ folder
  ffmpeg_path: "auto"       # "auto" = same resolution strategy
  default_format: "mp3"
  max_concurrent: 3

audio:
  fft_window_size: 4096     # Samples per FFT window (must be power of 2)
  fft_hop_size: 2048        # Samples between successive windows (50% overlap recommended)
  fft_rate_hz: 30           # Target FFT computation rate (clamped by hop_size / sample_rate)
  ring_buffer_size: 16384   # Capacity of the lock-free PCM ring buffer (audio→FFT)
```

### Built-in Themes

All 5 themes are hardcoded in `src/config/theme.rs`. Users can override or add custom themes in `config.yaml` under `theme.custom`.

#### 1. Synthwave (Default)
Retro 80s neon aesthetic — hot pinks, electric yellows, deep purple backgrounds.
```
Background: #0f0a1e     Surface: #1a1035     Primary: #f92a82     Secondary: #edfd09
Text: #ffffff           Text Dim: #7b6b8a    Accent: #00d9ff
Viz gradient:   #6b0f6b → #f92a82 → #edfd09 → #00d9ff
Viz glow:       ON       Bar style: Solid
Wave color:     #f92a82  Wave trail: #6b0f6b
Particle colors: [#f92a82, #edfd09, #00d9ff, #ff6b35]
```

#### 2. Dracula
Classic dark theme with soft pastels — purple, pink, cyan, green.
```
Background: #282a36     Surface: #44475a     Primary: #ff79c6     Secondary: #8be9fd
Text: #f8f8f2           Text Dim: #6272a4    Accent: #50fa7b
Viz gradient:   #6272a4 → #bd93f9 → #ff79c6 → #8be9fd
Viz glow:       ON       Bar style: Rounded
Wave color:     #50fa7b  Wave trail: #2d4a3e
Particle colors: [#ff79c6, #8be9fd, #bd93f9, #50fa7b, #ffb86c]
```

#### 3. Nord
Icy, minimal Scandinavian palette — muted blues, whites, subtle frost.
```
Background: #2e3440     Surface: #3b4252     Primary: #88c0d0     Secondary: #81a1c1
Text: #eceff4           Text Dim: #4c566a    Accent: #a3be8c
Viz gradient:   #4c566a → #5e81ac → #88c0d0 → #eceff4
Viz glow:       OFF      Bar style: Dots
Wave color:     #88c0d0  Wave trail: #3b4252
Particle colors: [#88c0d0, #81a1c1, #a3be8c, #ebcb8b]
```

#### 4. Tokyo Night
Warm, moody Japanese night — deep indigos, warm purples, soft oranges.
```
Background: #1a1b26     Surface: #24283b     Primary: #7aa2f7     Secondary: #bb9af7
Text: #c0caf5           Text Dim: #565f89    Accent: #9ece6a
Viz gradient:   #565f89 → #7aa2f7 → #bb9af7 → #ff9e64
Viz glow:       ON       Bar style: Solid
Wave color:     #7dcfff  Wave trail: #1a3a5c
Particle colors: [#7aa2f7, #bb9af7, #ff9e64, #9ece6a, #f7768e]
```

#### 5. Monochrome
Clean, minimal — whites, grays, and a single accent color. Lets the visualizer shape speak.
```
Background: #0a0a0a     Surface: #1a1a1a     Primary: #ffffff     Secondary: #888888
Text: #e0e0e0           Text Dim: #555555    Accent: #ffffff
Viz gradient:   #333333 → #666666 → #aaaaaa → #ffffff
Viz glow:       OFF      Bar style: Dots
Wave color:     #ffffff  Wave trail: #333333
Particle colors: [#ffffff, #cccccc, #999999, #666666]
```

### Environment Variables
- `RTUNES_CONFIG_PATH`: Override config location
- `RTUNES_LOG_LEVEL`: Override log level
- `RTUNES_LIBRARY_PATH`: Override/append music directory (comma-separated for multiple)

---

## Error Handling

### Error Types
```rust
#[derive(Debug, thiserror::Error)]
pub enum RtunesError {
    #[error("Config error: {0}")]
    Config(String),
    
    #[error("Audio playback error: {0}")]
    Audio(String),
    
    #[error("FFT processing error: {0}")]
    Visualizer(String),
    
    #[error("Download failed: {0}")]
    Fetcher(String),
    
    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}
```

### Error Display (TUI Mode)
- Non-critical: Toast notification at bottom (auto-dismiss after 5s)
- Critical: Modal dialog with "Press any key to continue"
- Log all errors to file with full context

---

## Logging

### Log Levels
- **ERROR**: Crashes, unrecoverable failures
- **WARN**: Missing files, degraded performance, skipped frames
- **INFO**: Track changes, downloads, config loads
- **DEBUG**: FFT buffer stats, frame times, thread events
- **TRACE**: Every render loop iteration (use sparingly)

### Log Format
```
2024-01-15T10:30:45.123Z [INFO] audio::player - Loaded track: "Song.mp3" {duration: 180s}
```

### Log Output
- **TUI mode**: Write to `~/.local/share/rtunes/rtunes.log` (Linux), `~/Library/Logs/rtunes/rtunes.log` (macOS), or `%APPDATA%\rtunes\rtunes.log` (Windows). Uses `tracing-appender::rolling::daily` to keep the last 7 days.
- **CLI mode**: Can optionally log to stderr if `--verbose`

---

## Visualizer Specifications

The visualizer is a **centerpiece feature** of RTunes. It should feel alive, reactive, and beautiful — something users would leave running as a screensaver. All visualizers use the active theme's `viz` palette for colors.

RTunes ships **10 distinct visualizers** organized into four families:

| # | Name | Family | Key | Source data | Identity |
|---|---|---|---|---|---|
| 1 | Spectrum | Bars | `1` | FFT bins | Linear frequency bars |
| 2 | Oscilloscope | Wave | `2` | Mono PCM | Time-domain waveform |
| 3 | Supernova | Bars | `3` | FFT bins | Radial spectrum (polar) |
| 4 | Particles | Physics | `4` | Band energies + beats | Chaotic point cloud |
| 5 | Aurora | Wave | `5` | Band energies | Layered sine wave parallax |
| 6 | Starfield | 3D | `6` | Band energies + beats | Perspective warp |
| 7 | Vectorscope | Wave | `7` | **Stereo** PCM (L vs R) | Lissajous phase plot — only stereo viz |
| 8 | Spectrogram | Bars | `8` | FFT bins (history) | Scrolling time-frequency waterfall — only viz with temporal history |
| 9 | Tunnel | 3D | `9` | Bass + beats | Concentric perspective rings |
| 10 | Matrix Rain | Glyph | `0` | Highs + beats | Falling glyph columns |

---

## Universal Smoothing & Animation Pipeline

Every visualizer pulls from the **same shared smoothing primitives** in `src/visualizer/smoothing.rs`. The goal: nothing on screen ever snaps, jitters, or looks "digital." The renderer runs at 60 FPS even when the underlying analysis runs at 30 Hz.

### Pipeline stages (in order, top of FFT thread)

1. **Hann window** on the time-domain buffer before FFT (already specified) — kills spectral leakage.
2. **Magnitude → log/dB** conversion: `mag_db = 20 * log10(mag.max(EPSILON))`. Linear FFT magnitudes are perceptually wrong; log scaling matches how humans hear loudness.
3. **Logarithmic frequency binning** (20 Hz → 20 kHz, mapped onto N bars where N = 32–64 depending on width).
4. **Spectral smoothing across bins** (3-tap weighted average `[0.25, 0.5, 0.25]`) — removes single-bin spikes that look like static.
5. **Asymmetric EMA** per bin:
   ```
   if new > prev: smoothed = mix(prev, new, ATTACK)   // ATTACK ≈ 0.5  (fast rise)
   else:          smoothed = mix(prev, new, RELEASE)  // RELEASE ≈ 0.15 (slow fall)
   ```
   Asymmetric coefficients keep transients punchy while preventing the bars from "twitching" downward. Different visualizers can override these constants.
6. **Peak hold with drift**: `peak[i] = max(smoothed[i], peak[i] * 0.96)` — produces the floating peak markers and works as input for "pulse" effects.
7. **One-Euro Filter** on slow-moving parameters (camera rotation in Supernova/Tunnel, target positions in Particles, color saturation). One-Euro adapts: high cutoff when value changes fast (snappy reaction), low cutoff when it's stable (no jitter). Implementation: ~30 lines of code, no deps.
8. **Spectral-flux beat detector**:
   ```
   flux = Σ max(0, mag[i] - prev_mag[i])
   adaptive_threshold = moving_avg(flux, 43 frames) * 1.5
   beat = flux > adaptive_threshold && now - last_beat > 250ms
   ```
   Sets `viz_data.beat = true` for exactly one frame and starts `beat_intensity` decaying from 1.0 with `0.92/frame`. Used by Particles, Tunnel, Starfield, Matrix Rain.
9. **Sub-frame interpolation** at the renderer: each frame gets `t = (now - viz_data.timestamp) / viz_data.fft_period`, clamped to `[0, 1]`. Visualizers `lerp(bins_prev, bins_smoothed, t)` per bar/point so movement is buttery between FFT updates.

### Render-side compositing tricks

- **Phosphor / motion blur** (Oscilloscope, Vectorscope): maintain a persistent "phosphor buffer" the size of the canvas. Each frame: multiply buffer by `0.85` (decay), then OR-paint the new shape on top. Renders that buffer instead of the raw shape. Produces an authentic CRT trail.
- **Glow halo** (when `viz.glow = true`): render the shape twice — once at full brightness in the primary color, once at 40% brightness offset by ±1 dot in a desaturated companion color. Cheap, looks like real bloom.
- **Sub-pixel positioning**: Braille's 2×4 dot grid means we have 8 logical pixels per terminal cell. All point/line plotting rounds to dot coordinates, not cell coordinates — the difference is dramatic for waves and tunnels.
- **Catmull-Rom spline interpolation** (Oscilloscope, Aurora): when sample count > pixel columns, sample as-is; when sample count < columns, interpolate with Catmull-Rom for round, organic curves rather than straight-line segments.
- **Temporal supersampling** (Particles, Starfield): render the previous frame at 60% brightness underneath the current frame. Doubles perceived motion smoothness essentially for free.

### Rendering Approach (general)

- All visualizers render onto a `ratatui::widgets::canvas::Canvas` using `Marker::Braille` for maximum resolution (2×4 dots per terminal cell = 8× the resolution of regular characters).
- Each visualizer is a `trait Visualizer { fn render(&mut self, frame, area, data, t, theme); }` so they're hot-swappable.

---

### 1. Spectrum (Linear Frequency Bars)
- **Style**: Vertical bars representing frequency bins — the canonical audio visualizer.
- **Bins**: Adaptive — 32 bars on narrow terminals (<60 cols), 48 mid (<120 cols), 64 on wide.
- **Colors**: Each bar uses the theme's `viz.gradient` color stops, interpolated by height (deep purple at bottom → hot pink at mid → cyan at top).
- **Bar style**: Controlled by theme — `Solid` (█), `Rounded` (▓▒░ gradient), or `Dots` (Braille).
- **Smoothing**: Universal pipeline (asymmetric EMA, peak hold, sub-frame interp). `ATTACK = 0.6`, `RELEASE = 0.15`.
- **Peak indicators**: Bright dot at `peak[i]`, drifting down at decay 0.96.
- **Mirror mode**: In fullscreen, optionally mirror bars downward with 50% opacity for a reflection-on-water effect.

### 2. Oscilloscope (Time-Domain Waveform)
- **Style**: Real-time waveform drawn with Braille — looks like an analog scope on phosphor.
- **Buffer**: Last 2048 mono PCM samples (`viz_data.pcm_mono`), plotted left-to-right.
- **Colors**: Primary waveform in `viz.wave_color`; phosphor buffer (decayed prior frames) in `viz.wave_trail`.
- **Smoothing**:
  - **Catmull-Rom spline** between sample points when stretched across a wide terminal (avoids pixelated zig-zag).
  - **Phosphor compositing** (decay 0.85) — gives the smear of a CRT.
  - **Zero-crossing trigger**: scan the buffer for the first rising zero-crossing and start drawing from there. Without this the wave drifts left/right every frame and looks unstable.
- **Glow**: Standard halo when `viz.glow` is on.
- **Centering**: Vertically centered, amplitude scaled to ~80% of available height with a soft clip (`tanh`-style) so loud peaks don't get squared off.

### 3. Supernova (Radial Spectrum)
- **Style**: Circular/radial frequency visualization expanding from screen center.
- **Bins**: 32 radial spokes evenly distributed around 360°.
- **Rendering**: Lines from center outward, length = `bins_smoothed[i] * R_max`.
- **Colors**: Spokes cycle through `viz.gradient` around the circle; the inner core pulses with `bass_energy` in `viz.particle_colors[0]`.
- **Pulse**: Inner radius `r0 = R_min + bass_energy * (R_min * 0.5)` — breathes with the bass.
- **Rotation**: Slowly rotates (0.3°/frame baseline + `mid_energy * 1.5°/frame` boost) — the rotation rate itself is fed through a One-Euro Filter so tempo changes feel natural.
- **Smoothing**: Universal pipeline + One-Euro on rotation angle (prevents jitter when mids are quiet).

### 4. Particles (Audio-Reactive Particle System)
- **Style**: 150–400 particles (count adaptive to terminal area) that respond to bands and beats.
- **Behavior**:
  - **Beat** (`viz_data.beat`) → spawn a 30-particle burst from center, outward velocity scaled by `beat_intensity`.
  - **Bass** → increase particle radius and add center-outward force.
  - **Mids** → add Perlin-noise-driven turbulence to velocities.
  - **Highs** → shift hue along `viz.particle_colors`.
- **Physics**: Sub-stepped Euler (2 substeps per render frame to avoid tunneling at high speeds):
  - Gentle gravity toward center, magnitude = `0.02 * dist_from_center`.
  - Velocity damping `0.97/frame`.
  - Bounded random jitter for organic feel.
- **Lifecycle**: `life ∈ [0, 1]`, decays at `0.005/frame`. Opacity = `life`. Dead particles are recycled (object pool, no allocations in render loop).
- **Smoothing**: Sub-stepped physics + temporal supersampling (60% prior frame underneath).
- **Rendering**: Each particle is a Braille dot. Bright when young, fading as it ages.

### 5. Aurora (Flowing Gradient Waves)
- **Style**: Flowing, layered sine waves like the Northern Lights — serene, atmospheric.
- **Layers**: 4 overlapping sine waves: `y_k(x, t) = A_k(t) * sin(ω_k * x + φ_k(t))` with different ω, φ, and per-layer color from `viz.gradient`.
- **Audio reactivity**:
  - `A_k(t)` modulated by `mid_energy` (per-layer offset so they don't all pulse together).
  - `dφ/dt` (scroll speed) modulated by `bass_energy`.
  - Color saturation scales with `loudness`.
- **Smoothing**:
  - All audio-driven parameters pass through One-Euro Filter — Aurora should *flow*, never twitch.
  - **Catmull-Rom spline** through sampled wave points for round curves.
- **Rendering**: Filled regions below each wave curve, alpha-blended where layers overlap (additive blending in HSL).
- **Movement**: Layers scroll horizontally at different speeds → parallax depth.

### 6. Starfield (3D Fly-Through)
- **Style**: Stars flying toward the viewer from a central vanishing point — warp-drive effect.
- **Stars**: 250–500 stars with `(x, y, z)` coordinates in a unit cube, recycled when `z < z_near`.
- **Perspective**: `screen_x = x / z`, `screen_y = y / z`. Brightness ∝ `1 / z` (close stars brighter).
- **Audio reactivity**:
  - Bass → forward speed (`dz/dt`); a beat momentarily multiplies speed by `1 + beat_intensity * 2` for a warp-jump effect.
  - Mids → spawn rate.
  - Highs → star color (sampled from `viz.gradient`).
- **Smoothing**:
  - Forward speed runs through One-Euro so warp transitions glide instead of snapping.
  - **Trail effect**: each star draws a Braille line from current to previous projected position (length scales with speed → real motion lines).

### 7. Vectorscope (Stereo Lissajous)
- **Style**: Classic broadcast-engineer X/Y plot of left vs right audio. The shape reveals stereo width, mono compatibility, and phase relationships — and looks beautiful while doing it.
- **Source**: `viz_data.pcm_stereo` — last 2048 stereo pairs. **This is the only visualizer that uses raw stereo.**
- **Mapping**:
  - Standard rotation: `x = (L - R) * scale`, `y = (L + R) * scale` (rotates 45° so a mono signal traces a vertical line and pure stereo traces a horizontal line — the audio engineer's convention).
  - Centered in the canvas.
- **Phosphor compositing** (decay 0.88): the scope buffer accumulates over time, giving the classic glowing-electron-beam smear. This is the heart of the look.
- **Colors**: Beam in `viz.wave_color`; phosphor decay tints toward `viz.wave_trail`.
- **Glow**: Halo on when `viz.glow` is enabled.
- **Audio reactivity**:
  - Beam intensity modulated by `loudness` so quiet passages don't blow out the phosphor.
  - On a beat, the beam briefly thickens (2-pixel pen instead of 1-pixel).
- **Smoothing**: Phosphor buffer is the smoothing — no per-sample EMA needed. Catmull-Rom spline between sample points to keep the trace smooth at low sample rates.

### 8. Spectrogram (Scrolling Waterfall)
- **Style**: A scrolling time-frequency map. Frequency on the X axis (log scale), time on the Y axis (newest at top, scrolling down). Cell color = magnitude.
- **Source**: `viz_data.spectrogram_rows` — a `VecDeque` of past FFT frames, length = terminal height. Each frame, push the new bins on top, drop the oldest from the bottom.
- **Colors**: Magnitude → color via `viz.gradient` interpolation. Below threshold renders as background (transparent).
- **Smoothing**:
  - Each *row* is the EMA-smoothed bins from that timestep — so transient noise doesn't stripe horizontally.
  - **Vertical lerp** during sub-frame interpolation: when `t < 1.0`, the top row is rendered as a partial line whose color is `lerp(prev_top, current_top, t)` — produces silky scrolling at 60 FPS even though new rows only arrive at 30 Hz.
  - Optional **Gaussian blur** (3×3 kernel, σ ≈ 0.8) over the full waterfall for the soft "weather radar" look. Cheap if implemented as separable horizontal+vertical passes.
- **Distinctness**: This is the only visualizer with **temporal history** — you can literally see the chord progression over the last several seconds.
- **Modes** (cyclable with `m` while spectrogram active): standard / inverted (low freq right) / mirrored (centered, freq spreads outward).

### 9. Tunnel (Concentric Perspective Rings)
- **Style**: Vortex/warp tunnel of concentric rings receding into the distance, pulsing with the music. Hypnotic.
- **Geometry**:
  - Render N rings (12–20) at exponentially increasing depths `z_k = z_0 * 1.3^k`.
  - Each ring is an ellipse in screen space: `(x, y) = (cos θ, sin θ) * R / z_k`, sampled at 64 angular points.
  - Optional twist: each ring is rotated by `phase_k = base_phase + k * twist_per_ring` — produces a spiral when twist > 0.
- **Audio reactivity**:
  - **Bass** → camera "thrust": all rings shift toward the viewer (decrement `z`). When a ring reaches `z_near`, it's recycled to the back. On a beat, an extra speed pulse.
  - **Mids** → ring deformation: `R_k(θ) = R_base * (1 + 0.15 * sin(8θ + audio_phase))` — rings ripple in a flower-like pattern.
  - **Highs** → ring color shimmer along `viz.gradient`.
- **Smoothing**:
  - Camera thrust uses One-Euro Filter — bass dropouts don't stutter the camera.
  - Ring radii sub-frame-interpolated.
  - Phosphor decay (0.7) on the ring trail buffer for the "speed lines" look.
- **Colors**: Inner rings (close) bright, outer rings (far) dim — value mapped through `viz.gradient` by depth.

### 10. Matrix Rain (Falling Glyph Columns)
- **Style**: The iconic green digital rain — but reactive to the music. Pure character art (no Braille canvas), giving it a different texture from every other viz.
- **Geometry**:
  - Each terminal column is a "stream" with: `head_y` (current bottom of the trail), `length` (5–25 cells), `speed` (0.5–2.0 cells/frame), `glyphs[]` (random Katakana / box-drawing / ASCII picked at spawn).
  - Head cell = brightest (`viz.primary`); body fades along `viz.gradient`; tail end blends to background.
- **Audio reactivity**:
  - **Highs** → glyph mutation rate (cells in the stream randomly swap to new glyphs more often when highs are loud — produces a "data crackle" feel).
  - **Mids** → fall speed multiplier.
  - **Bass / beats** → on a beat, a few random columns flash full-bright (`beat_intensity` brightness boost) and a new wave of fast-falling streams spawns.
  - **Loudness** → average stream density (more streams when loud, sparser when quiet).
- **Smoothing**:
  - Speed and density modulated through One-Euro — the rain shouldn't stutter on quiet bridges.
  - Sub-frame interp on `head_y` so cells fall smoothly even though the FFT updates at 30 Hz.
- **Distinctness**: The only visualizer that's pure glyph-driven — the "negative space" of the visualizer family. Particularly stunning in the Monochrome theme.

---

### Fullscreen Visualizer Mode
- Press `f` to toggle — the visualizer takes the **entire terminal area**.
- In fullscreen, a subtle overlay shows: track name + artist (top-left, dim text, auto-hides after 3 seconds, reappears on track change).
- Progress bar rendered as a thin line at the very bottom edge (1 row).
- The hint bar is hidden in fullscreen to maximize visual space.
- **Screensaver worthy** — designed to be left running.

---

## Performance Requirements

### Targets
- **Startup time**: <300ms (cold start)
- **Memory**: <100MB baseline, <200MB with visualizers active
- **CPU**: <8% on modern CPUs (2020+), single-core bound
- **Frame rate**: Stable 60 FPS in most terminals, graceful degradation to 30 FPS

### Optimizations
- Use `Marker::Braille` for 2x4 resolution boost in canvas rendering
- Bounded channels with `try_recv()` to drop stale FFT frames
- Minimize allocations in hot paths (render loop, FFT processing)
- Profile with `cargo flamegraph` to find bottlenecks

---

## Testing Strategy

### Unit Tests
**Location**: `#[cfg(test)]` modules in each file  
**Coverage**: >70% for core logic  
**Focus**:
- FFT binning with synthetic sine waves (440Hz, 880Hz)
- Config parsing with invalid YAML
- Track metadata extraction with corrupted files

### Integration Tests
**Location**: `tests/integration/`  
**Tests**:
1. Inject a fake yt-dlp via the `Fetcher` trait (see below), verify file appears in library and `FetchEvent::Done` fires.
2. Simulate keyboard events through the event handler, verify `AppState` mutations.
3. Load 1000-track library, measure scan time (regression-guard scan throughput).

### Test Seams
To keep the audio + fetcher boundaries testable without real subprocesses or sound devices:

```rust
pub trait Fetcher: Send + Sync {
    fn fetch(&self, url: &Url, opts: &FetchOpts, tx: Sender<FetchEvent>) -> Result<()>;
}

pub trait AudioBackend: Send {
    fn play(&mut self, path: &Path) -> Result<()>;
    fn pause(&mut self);
    fn resume(&mut self);
    fn seek(&mut self, position: Duration) -> Result<()>;
    fn position(&self) -> Duration;
}
```

- Production: `YtDlpFetcher` (real subprocess) and `RodioBackend` (real `OutputStream`).
- Tests: `MockFetcher` (writes a stub file + emits scripted events) and `SilentBackend` (counts samples without producing sound).
- `OutputStream::try_default()` failures in CI are handled by the silent-mode fallback (see Threading Architecture), so headless test environments don't need an audio device.

### Manual Testing Checklist
- [ ] Visualizers render correctly in: Windows Terminal, Alacritty, iTerm2, Kitty
- [ ] Resize terminal during playback (no panic)
- [ ] Play 10+ tracks in sequence (no memory leak)
- [ ] Download invalid URL (graceful error)
- [ ] Handle corrupted audio file (skip with warning)

---

## Build & Distribution

### Development
```bash
cargo run -- tui --theme dracula
cargo run -- fetch "https://youtube.com/watch?v=..." --format flac
```

### Release Build
```bash
cargo build --release
# strip is configured in Cargo.toml [profile.release] — no manual strip needed
```

### Binary Outputs
- **Linux**: `target/release/rtunes`
- **macOS**: `target/release/rtunes`
- **Windows**: `target/release/rtunes.exe`

### Distribution
```
rtunes-v0.1.0-windows/
├── rtunes.exe
├── deps/
│   ├── yt-dlp.exe
│   ├── ffmpeg.exe
│   └── ffprobe.exe
└── README.md
```
Zip the folder and share. Users with yt-dlp/ffmpeg in PATH don't need the deps/ folder.

---

## Security Considerations

### URL Sanitization
```rust
// Validate URL before passing to yt-dlp
fn validate_url(url: &str) -> Result<Url> {
    let parsed = Url::parse(url)?;
    if !["http", "https"].contains(&parsed.scheme()) {
        return Err(RtunesError::Fetcher("Invalid scheme".into()));
    }
    // Prevent command injection via malformed URLs
    // Note: Rust's Command API already prevents shell injection since it
    // passes arguments directly (not via shell), but we validate anyway
    if url.contains(';') || url.contains('|') {
        return Err(RtunesError::Fetcher("Suspicious characters".into()));
    }
    // Note: '&' is valid in YouTube URLs (query params), so we don't reject it
    Ok(parsed)
}
```

### Path Traversal Prevention
```rust
// Ensure library paths stay within configured directory
fn safe_path(base: &Path, relative: &Path) -> Result<PathBuf> {
    let canonical = base.join(relative).canonicalize()?;
    if !canonical.starts_with(base) {
        return Err(RtunesError::Io("Path traversal attempt".into()));
    }
    Ok(canonical)
}
```

---

## Implementation Priority

### Phase 1: Core Audio (Week 1-2)
1. Project setup, dependency configuration
2. CLI argument parsing with clap
3. Config file loading (YAML) with `dirs` crate for cross-platform paths
4. Library scanner (find audio files, extract metadata with `lofty`)
5. Basic audio playback with rodio + TapSource PCM interception (headless, no TUI)

### Phase 2: TUI Foundation (Week 3-4)
1. Ratatui + crossterm setup
2. Basic layout: library list, now playing, controls
3. Keyboard input handling
4. State management with Arc<Mutex<AppState>>
5. Play/pause/next/prev functionality

### Phase 3: Visualizer Foundation (Week 5-6)
1. TapSource → ArrayQueue<StereoSample> → FFT thread pipeline
2. FFT thread with rustfft + Hann windowing + log-magnitude binning
3. **Universal smoothing module** (`smoothing.rs`): asymmetric EMA, peak hold, One-Euro Filter, spectral-flux beat detector, sub-frame interpolation helper
4. Canvas rendering with Braille; phosphor buffer + glow halo helpers
5. Spectrum + Oscilloscope visualizers
6. Visualizer cycling (`v` / `V` / `1`–`0` direct jumps)

### Phase 4: Visualizer Variety (Week 7-8)
1. Supernova (radial bars)
2. Particles (physics + beat reactivity)
3. Aurora (layered waves with One-Euro)
4. Starfield (3D fly-through with warp jumps on beats)
5. Vectorscope (stereo Lissajous with phosphor)
6. Theme system + glow toggle (`g`)
7. Fullscreen mode toggle (`f`)

### Phase 5: Advanced Visualizers + Downloads (Week 9-10)
1. Spectrogram (scrolling waterfall + sub-frame vertical lerp)
2. Tunnel (concentric perspective rings + beat thrust)
3. Matrix Rain (glyph-driven, only character-based viz)
4. yt-dlp wrapper with progress tracking
5. Download queue management
6. Error toast notifications
7. Performance profiling (`cargo flamegraph`); ensure 60 FPS holds with each viz
8. Documentation and README

---

## Code Style Guidelines

### Naming Conventions
- **Functions/variables**: `snake_case`
- **Types/structs/enums**: `PascalCase`
- **Constants**: `SCREAMING_SNAKE_CASE`
- **Modules**: `snake_case`

### Documentation
```rust
/// Computes FFT on the given audio buffer.
///
/// # Arguments
/// * `samples` - Raw PCM samples (f32, mono)
/// * `window` - Windowing function (Hann/Hamming)
///
/// # Returns
/// Normalized frequency bins (0.0-1.0)
///
/// # Example
/// ```
/// let bins = compute_fft(&samples, WindowType::Hann)?;
/// ```
pub fn compute_fft(samples: &[f32], window: WindowType) -> Result<Vec<f32>>
```

### Error Handling
- Use `anyhow` for application errors with context
- Use `thiserror` for library error types
- Always propagate errors with `?` operator
- Log errors before returning

---

## Examples

### Example 1: Launch TUI
```bash
$ rtunes tui --theme synthwave --fps 60
# Terminal switches to alternate screen
# Library loads with visualizer running
```

### Example 2: Download Track
```bash
$ rtunes fetch "https://youtube.com/watch?v=dQw4w9WgXcQ" --format opus
[INFO] Downloading: "Never Gonna Give You Up"
[====================================] 100% (3.2 MB)
[INFO] Saved to ~/Music/Never Gonna Give You Up.opus
```

### Example 3: Rebuild Library
```bash
$ rtunes scan
Scanning 2 library folders...
  ~/Music — 215 tracks
  ~/Downloads/Music — 32 tracks
Found 247 tracks total.
Library index updated.
```

### Example 4: Manage Library Folders
```bash
$ rtunes library list
Library folders:
  1. ~/Music (215 tracks)

$ rtunes library add "D:/My Songs"
Added: D:/My Songs
Reindexing... Found 312 tracks total.

$ rtunes library remove "~/Music"
Removed: ~/Music
Reindexing... Found 97 tracks total.
```

---

## Edge Cases

### Edge Case 1: Terminal Resize During Render
**Scenario**: User resizes terminal while visualizer is running  
**Handling**:
1. Catch `crossterm::event::Event::Resize` event
2. Pause rendering for current frame
3. Recalculate layout chunks with new dimensions
4. Adjust FFT bin count to match new width
5. Resume rendering

### Edge Case 2: Extremely Long Track (>2 hours)
**Scenario**: User plays audiobook or DJ mix  
**Handling**:
- Use `u64` for duration (seconds)
- Format progress as `HH:MM:SS` instead of `MM:SS`
- Ensure seek bar precision doesn't overflow

### Edge Case 3: Empty Library
**Scenario**: User launches TUI with no music files  
**Handling**:
- Display placeholder: "No tracks found. Press 'd' to download or 'a' to add a library folder."
- Also suggest: "Press Ctrl+L to open Library Manager"
- Disable play controls
- Visualizer shows idle animation

### Edge Case 4: yt-dlp Not Found
**Scenario**: User tries to download without yt-dlp installed or in deps/ folder  
**Handling**:
1. Check PATH first, then exe-adjacent `deps/` folder
2. If not found anywhere, show error toast in TUI: "yt-dlp not found. Place it in the `deps/` folder next to rtunes.exe, or install it to PATH."
3. Don't crash, return to normal mode

### Edge Case 5: Library Manager — Remove Last Folder
**Scenario**: User removes the last remaining library folder via Library Manager  
**Handling**:
- Allow removal (don't prevent it)
- Show toast: "All library folders removed. Press 'a' to add a new one."
- Library clears, shows empty state placeholder
- Config is saved immediately

### Edge Case 6: Rescan During Playback
**Scenario**: User presses `R` to rescan while a track is playing  
**Handling**:
- Rescan runs in a background thread, does NOT interrupt playback
- Show toast: "Rescanning library..."
- On completion: "Rescan complete. Found X tracks."
- Currently playing track remains playing even if it was re-indexed
- The `current_index` is updated to point to the same track in the new library list

---

## Future Enhancements
- [ ] Playlist support (M3U/PLS)
- [ ] Equalizer (10-band)
- [ ] Gapless playback
- [ ] Album art display (with sixel/kitty graphics)
- [ ] Discord Rich Presence integration
- [ ] Global media key support
- [ ] Crossfade between tracks
- [ ] Smart shuffle (avoid artist repetition)

---

## Notes for AI Code Generation

### Critical Implementation Details

#### Integrated Rendering — Visualizer as Background

The visualizer is NOT a boxed widget. It is a **full-screen background layer** that renders
behind all other UI elements. The library, controls, and header are translucent overlays
on top. This creates a seamless, immersive feel where the visualizer IS the app.

```rust
// In render function — the visualizer ALWAYS fills the full terminal
let full_area = frame.area();

// STEP 1: Render visualizer across the ENTIRE terminal (background layer)
render_visualizer(frame, full_area, &viz_data, sub_frame_t, &theme);

// STEP 2: Overlay UI panels on top (semi-transparent / dim backgrounds)
if !app_state.is_fullscreen {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),      // Now playing bar (minimal)
            Constraint::Min(10),        // Open space (visualizer shows through)
            Constraint::Length(8),      // Library list (translucent overlay)
            Constraint::Length(2),      // Controls + progress bar
            Constraint::Length(1),      // Hint bar
        ])
        .split(full_area);

    render_now_playing(frame, chunks[0], &app_state, &theme);
    // chunks[1] is intentionally EMPTY — visualizer shows through
    render_library_overlay(frame, chunks[2], &app_state, &theme);
    render_controls(frame, chunks[3], &app_state, &theme);
    render_hint_bar(frame, chunks[4], &app_state, &theme);
} else {
    // Fullscreen: only a subtle track name that fades after 3s
    render_fullscreen_overlay(frame, full_area, &app_state, &theme);
}

// STEP 3: Modal overlays (on top of everything, mutually exclusive)
if app_state.show_help {
    render_help_overlay(frame, full_area);
} else if app_state.show_library_manager {
    render_library_manager(frame, full_area, &app_state, &theme);
}
```

#### UI Panel Styling — Blending In

All overlay panels follow these rules to feel integrated rather than bolted on:

- **No hard box borders**. Use `Borders::NONE` or at most a single thin line separator
  using `theme.surface` color (which is close to background, not a contrasting border).
- **Dim backgrounds**. Library and controls panels use the `theme.background` color
  but at a slightly lighter shade (`theme.surface`) so they're distinguishable without
  being jarring. They look like frosted glass over the visualizer.
- **Text blends with visualizer colors**. The "now playing" bar uses `theme.primary`
  for the track title — the same primary color used in the visualizer gradient —
  so it feels like part of the same visual language.
- **Progress bar matches visualizer**. The seek/progress bar uses a gradient that
  matches the current visualizer's `viz.gradient` colors, not a flat solid color.
- **Smooth transitions**. When switching between normal and fullscreen mode, the UI
  panels don't snap in/out — they could fade (by rendering with progressively dimmer
  text over 3-4 frames).

#### Layout in Normal Mode (Visualizer Behind Everything)

```
┌─────────────────────────────────────────────────────┐
│ ♫ Track Title — Artist                    Spectrum ▸│ 1 row — now playing (dim bg)
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│ ← visualizer fills
│▒▒▒▒▒▒▒▒▒▒ VISUALIZER (full background) ▒▒▒▒▒▒▒▒▒▒│    this space
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
│ ▸ Song A — Artist 1                         3:45   │
│   Song B — Artist 2                         4:12   │ 8 rows — library
│   Song C — Artist 3                         2:58   │ (translucent overlay)
│   Song D — Artist 4                         5:01   │
│ ━━━━━━━━━━━━━━━●━━━━━━━━━  2:34 / 3:45  🔊 75%    │ 2 rows — controls
│ [Space]Pause [n/p]Track [←→]Seek [?]Help [q]Quit   │ 1 row  — hint bar
└─────────────────────────────────────────────────────┘
```

#### Layout in Fullscreen Mode

```
┌─────────────────────────────────────────────────────┐
│ ♫ Track — Artist              (fades after 3s)      │
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
│▒▒▒▒▒▒▒▒▒ VISUALIZER (entire screen) ▒▒▒▒▒▒▒▒▒▒▒▒▒│
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
│▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒▒│
│ ━━━━━━━━━━━━━━━━━━━━━━●━━━━━━━━━━━━━━━━━━━━━━━━━━ │ thin progress line
└─────────────────────────────────────────────────────┘
```
```

#### FFT Windowing
```rust
// Apply Hann window before FFT to reduce spectral leakage
fn apply_hann_window(samples: &mut [f32]) {
    let n = samples.len();
    for (i, sample) in samples.iter_mut().enumerate() {
        let window = 0.5 * (1.0 - f32::cos(2.0 * PI * i as f32 / n as f32));
        *sample *= window;
    }
}
```

#### Render Loop Timing
```rust
use std::time::{Duration, Instant};

let frame_duration = Duration::from_millis(1000 / fps);

loop {
    let frame_start = Instant::now();
    
    // Handle events (non-blocking)
    while crossterm::event::poll(Duration::ZERO)? {
        if let Event::Key(key) = crossterm::event::read()? {
            handle_input(key, &app_state)?;
        }
    }
    
    // Render frame
    let sub_frame_t = compute_sub_frame_t(&viz_data);  // 0.0..1.0 between FFT frames
    terminal.draw(|frame| ui::render(frame, &app_state, &viz_data, sub_frame_t))?;
    
    // Sleep for remaining frame budget (skip if frame took too long)
    let elapsed = frame_start.elapsed();
    if elapsed < frame_duration {
        std::thread::sleep(frame_duration - elapsed);
    }
}
```

#### Cross-Platform Config Paths
```rust
use dirs;

fn config_path() -> PathBuf {
    // Uses the `dirs` crate for correct cross-platform behavior:
    // Windows: %APPDATA%\rtunes\config.yaml
    // Linux:   ~/.config/rtunes/config.yaml
    // macOS:   ~/Library/Application Support/rtunes/config.yaml
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rtunes")
        .join("config.yaml")
}
```

#### Terminal Cleanup on Panic

A panic mid-render leaves the user's terminal in raw mode + alternate screen — broken until they close it. Install a panic hook **before** entering raw mode so any panic restores the terminal first, then prints the panic message:

```rust
fn install_panic_hook() {
    let original = std::panic::take_hook();
    std::panic::set_hook(Box::new(move |info| {
        // Best-effort terminal restore — ignore errors here, we're already panicking.
        let _ = crossterm::terminal::disable_raw_mode();
        let _ = crossterm::execute!(
            std::io::stdout(),
            crossterm::terminal::LeaveAlternateScreen,
            crossterm::cursor::Show,
        );
        original(info);
    }));
}
```

Call `install_panic_hook()` as the very first line of `main`, before any TUI setup.

### Quality Checks
- [ ] No `unwrap()` in production code (use `?` or `unwrap_or_default()`)
- [ ] All public APIs documented
- [ ] Error messages are actionable
- [ ] No busy-wait loops (use sleep-based frame budgeting or blocking channels)
- [ ] Panic hook installed before entering raw mode (see above) — terminal always restores on crash
- [ ] Graceful shutdown also covered by `Drop` on a `TerminalGuard` RAII wrapper for the normal-exit path
- [ ] Memory profiles show no leaks over 1-hour runtime
- [ ] All file paths run through `shellexpand::tilde` on read and `dunce::canonicalize` for comparison/dedup