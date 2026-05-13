//! Shared application state and core data models for the TUI and audio thread.
//!
//! [`AppState`] is the single source of truth for library contents, input mode, player
//! fields, and overlay flags. It is normally wrapped in `Arc<Mutex<AppState>>`.

use std::path::{Path, PathBuf};
use std::str::FromStr;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;

use crate::config::RtunesConfig;
use crate::config::Theme;
use crate::fetcher::{FetchOpts, MissingTool};
use crate::utils::expand_path;

/// One indexed audio file (metadata from Lofty when available).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Track {
    pub id: String,
    pub filepath: PathBuf,
    pub title: String,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub duration_secs: u64,
}

/// Stable SHA-256 (hex) of the canonical file path bytes.
pub fn track_id_for_path(path: &Path) -> String {
    let bytes: Vec<u8> = dunce::canonicalize(path)
        .map(|p| p.as_os_str().to_string_lossy().as_bytes().to_vec())
        .unwrap_or_else(|_| path.as_os_str().to_string_lossy().as_bytes().to_vec());
    let digest = Sha256::digest(&bytes);
    hex::encode(digest)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum RepeatMode {
    #[default]
    Off,
    All,
    One,
}

impl RepeatMode {
    pub fn from_config_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "all" => Self::All,
            "one" => Self::One,
            _ => Self::Off,
        }
    }

    pub fn as_config_str(self) -> &'static str {
        match self {
            Self::Off => "off",
            Self::All => "all",
            Self::One => "one",
        }
    }
}

/// Spectrogram display layout (cycled with `Shift+M` while Spectrogram is active).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SpectrogramMode {
    #[default]
    Standard,
    Inverted,
    Mirrored,
}

impl SpectrogramMode {
    pub fn from_config_str(s: &str) -> Self {
        match s.trim().to_lowercase().as_str() {
            "inverted" => Self::Inverted,
            "mirrored" => Self::Mirrored,
            _ => Self::Standard,
        }
    }

    pub fn as_config_str(self) -> &'static str {
        match self {
            Self::Standard => "standard",
            Self::Inverted => "inverted",
            Self::Mirrored => "mirrored",
        }
    }
}

/// Active full-screen spectrum / oscilloscope / … renderer in the TUI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VisualizerMode {
    Spectrum,
    Spectrogram,
    Oscilloscope,
    Vectorscope,
    Supernova,
    PulseRings,
    BandMeter,
    Particles,
}

impl std::fmt::Display for VisualizerMode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Spectrum => "spectrum",
            Self::Spectrogram => "spectrogram",
            Self::Oscilloscope => "oscilloscope",
            Self::Vectorscope => "vectorscope",
            Self::Supernova => "supernova",
            Self::PulseRings => "pulse_rings",
            Self::BandMeter => "band_meter",
            Self::Particles => "particles",
        })
    }
}

impl FromStr for VisualizerMode {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match s.trim().to_lowercase().replace('-', "_").as_str() {
            "spectrum" => Ok(Self::Spectrum),
            "spectrogram" => Ok(Self::Spectrogram),
            "oscilloscope" => Ok(Self::Oscilloscope),
            "vectorscope" => Ok(Self::Vectorscope),
            "supernova" => Ok(Self::Supernova),
            "pulse_rings" => Ok(Self::PulseRings),
            "band_meter" => Ok(Self::BandMeter),
            "particles" => Ok(Self::Particles),
            _ => Err(format!("unknown visualizer: {s}")),
        }
    }
}

/// Library + chrome layout when not in fullscreen (`is_fullscreen`).
///
/// Full-screen viz mode is [`AppState::is_fullscreen`]; it hides the library and uses the
/// fullscreen layout — cycle with **Tab** or toggle with **`f`**.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanelFocus {
    /// Library pane (left) and visualizer pane (right); transport below.
    #[default]
    Normal,
    /// Visualizer fills the body; transport remains.
    TransportOnly,
}

/// Text-input overlays (search, URL download, add folder, library manager, settings).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InputMode {
    Normal,
    Search,
    DownloadUrl,
    AddLibraryPath,
    LibraryManager,
    Settings,
}

/// Which row is focused inside the Settings overlay.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum SettingsRow {
    #[default]
    YtDlp,
    Ffmpeg,
    DownloadDir,
}

/// Playback fields mirrored from the audio thread (position, repeat, shuffle, etc.).
#[derive(Debug, Clone)]
pub struct PlayerState {
    pub is_playing: bool,
    pub volume: f32,
    pub muted: bool,
    pub current_index: Option<usize>,
    pub position_secs: f64,
    pub duration_secs: f64,
    pub seek_to: Option<f64>,
    pub shuffle: bool,
    pub repeat: RepeatMode,
    /// Set by the TUI to ask the audio thread to reconnect the output device immediately.
    pub force_reconnect: bool,
    /// Written by the audio thread; `true` when no output device is available.
    pub silent_mode: bool,
}

impl Default for PlayerState {
    fn default() -> Self {
        Self {
            is_playing: false,
            volume: 1.0,
            muted: false,
            current_index: None,
            position_secs: 0.0,
            duration_secs: 0.0,
            seek_to: None,
            shuffle: false,
            repeat: RepeatMode::Off,
            force_reconnect: false,
            silent_mode: false,
        }
    }
}

/// Configured library folder with cached metadata.
#[derive(Debug, Clone)]
pub struct LibraryFolder {
    pub path: PathBuf,
    pub track_count: usize,
    pub last_scanned: Option<Instant>,
}

/// Central application state (normally `Arc<Mutex<AppState>>` in the TUI + audio worker).
#[derive(Debug)]
pub struct AppState {
    pub library: Vec<Track>,
    pub filtered_indices: Vec<usize>,
    pub search_query: Option<String>,
    pub input_mode: InputMode,
    pub input_buffer: String,
    pub player: PlayerState,
    pub visualizer_mode: VisualizerMode,
    pub is_fullscreen: bool,
    /// Library card visibility when not [`is_fullscreen`](Self::is_fullscreen).
    pub panel_focus: PanelFocus,
    pub neon_enabled: bool,
    pub spectrogram_mode: SpectrogramMode,
    pub selected_track: usize,
    pub message: Option<(String, Instant)>,
    pub download_progress: Option<f32>,
    /// Short status from yt-dlp (e.g. stage line) for the controls row.
    pub download_stage: Option<String>,
    pub show_help: bool,
    pub show_library_manager: bool,
    pub library_folders: Vec<LibraryFolder>,
    pub selected_folder: usize,
    pub is_rescanning: bool,
    /// A rescan was requested while one was already running; fire another when the current one finishes.
    pub rescan_pending: bool,
    pub show_settings: bool,
    pub settings_row: SettingsRow,
    /// Mirrors `config.fetcher.ytdlp_path`; kept in sync when settings change.
    pub settings_ytdlp_value: String,
    /// Mirrors `config.fetcher.ffmpeg_path`; kept in sync when settings change.
    pub settings_ffmpeg_value: String,
    /// Mirrors `config.app.download_dir`; kept in sync when settings change.
    pub settings_download_dir: String,
    pub quit: bool,
    /// When `Some`, the user is being asked whether to auto-download missing deps.
    /// The Vec lists the tools that are missing.
    pub deps_prompt: Option<Vec<MissingTool>>,
    /// URL + opts waiting to be submitted once deps are available.
    pub pending_fetch: Option<(Url, FetchOpts)>,
}

impl AppState {
    pub fn new(config: &RtunesConfig, _resolved_theme: Theme) -> Self {
        let visualizer_mode = match config.app.default_visualizer.parse::<VisualizerMode>() {
            Ok(v) => v,
            Err(_) => {
                tracing::warn!(
                    viz = %config.app.default_visualizer,
                    "unknown default_visualizer; using spectrum"
                );
                VisualizerMode::Spectrum
            }
        };

        let library_folders: Vec<LibraryFolder> = config
            .app
            .library_paths
            .iter()
            .map(|p| LibraryFolder {
                path: expand_path(p),
                track_count: 0,
                last_scanned: None,
            })
            .collect();

        let player = PlayerState {
            volume: config.app.volume.clamp(0.0, 1.0),
            shuffle: config.app.shuffle,
            repeat: RepeatMode::from_config_str(&config.app.repeat),
            ..Default::default()
        };

        Self {
            library: Vec::new(),
            filtered_indices: Vec::new(),
            search_query: None,
            input_mode: InputMode::Normal,
            input_buffer: String::new(),
            player,
            visualizer_mode,
            is_fullscreen: config.app.start_fullscreen,
            panel_focus: PanelFocus::default(),
            neon_enabled: config.app.neon,
            spectrogram_mode: SpectrogramMode::from_config_str(&config.app.spectrogram_mode),
            selected_track: 0,
            message: None,
            download_progress: None,
            download_stage: None,
            show_help: false,
            show_library_manager: false,
            library_folders,
            selected_folder: 0,
            is_rescanning: false,
            rescan_pending: false,
            show_settings: false,
            settings_row: SettingsRow::default(),
            settings_ytdlp_value: config.fetcher.ytdlp_path.clone(),
            settings_ffmpeg_value: config.fetcher.ffmpeg_path.clone(),
            settings_download_dir: config.app.download_dir.clone(),
            quit: false,
            deps_prompt: None,
            pending_fetch: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[test]
    fn track_id_stable_for_same_file() {
        let dir = std::env::temp_dir().join(format!("rtunes-track-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("fixture.mp3");
        std::fs::File::create(&p).unwrap().write_all(b"x").unwrap();
        let id1 = track_id_for_path(&p);
        let id2 = track_id_for_path(&p);
        assert_eq!(id1, id2);
        assert_eq!(id1.len(), 64);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
