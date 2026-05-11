//! Configuration paths, load/save, and theme resolution.

pub mod theme;

use std::collections::HashMap;
use std::fs;
use std::io::Write;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::error::{Result, RtunesError};

pub use theme::{resolve_active_theme, Theme};

const CONFIG_ENV: &str = "RTUNES_CONFIG_PATH";

const DEFAULT_CONFIG_YAML: &str =
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/assets/default_config.yaml"));

/// Parent directory of the running executable, if `current_exe` succeeds.
fn exe_parent_dir() -> Option<PathBuf> {
    std::env::current_exe()
        .ok()
        .and_then(|p| p.parent().map(Path::to_path_buf))
}

/// Default config file location, with two-tier resolution:
///
/// 1. `RTUNES_CONFIG_PATH` env var (highest priority).
/// 2. **Portable mode**: if `config.yaml` exists next to the executable, use it.
///    This preserves behaviour for development builds and self-contained archives.
/// 3. **Installed mode**: `<OS config dir>/rtunes/config.yaml`
///    (`%APPDATA%\rtunes\` on Windows, `~/.config/rtunes/` on Linux,
///    `~/Library/Application Support/rtunes/` on macOS).
/// 4. Falls back to `./config.yaml` if none of the above can be resolved.
pub fn config_path() -> PathBuf {
    if let Ok(p) = std::env::var(CONFIG_ENV) {
        return PathBuf::from(p);
    }
    // Portable override: exe-adjacent config.yaml takes priority when it already exists.
    if let Some(exe_cfg) = exe_parent_dir().map(|d| d.join("config.yaml")) {
        if exe_cfg.exists() {
            return exe_cfg;
        }
    }
    // Installed mode: OS standard config directory.
    dirs::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rtunes")
        .join("config.yaml")
}

/// `--config` wins when set; otherwise [`config_path`].
pub fn resolved_config_path(cli_override: Option<&Path>) -> PathBuf {
    cli_override
        .map(Path::to_path_buf)
        .unwrap_or_else(config_path)
}

/// Directory for rolling log files (`tracing-appender`).
pub fn log_dir() -> PathBuf {
    dirs::data_local_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("rtunes")
}

/// Ensures parent of default [`config_path`] and log dir exist.
#[allow(dead_code)]
pub fn ensure_dirs() -> std::io::Result<()> {
    ensure_dirs_for(&config_path())
}

/// Ensures parent of `resolved_config_path` and log dir exist.
pub fn ensure_dirs_for(resolved_config_path: &Path) -> std::io::Result<()> {
    if let Some(parent) = resolved_config_path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::create_dir_all(log_dir())?;
    Ok(())
}

// --- Structured config (serde_yaml) ---

fn default_volume() -> f32 {
    0.7
}

fn default_repeat_str() -> String {
    "off".to_string()
}

fn default_neon() -> bool {
    true
}

fn default_spectrogram_mode_str() -> String {
    "standard".to_string()
}

fn default_pcm_snapshot_samples() -> u32 {
    1024
}

fn default_spectrogram_history_rows() -> u32 {
    64
}

fn default_particles_substeps() -> u32 {
    1
}

fn default_particles_max() -> u32 {
    250
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AppSettings {
    pub library_paths: Vec<String>,
    pub download_dir: String,
    pub fps: u8,
    pub default_visualizer: String,
    pub start_fullscreen: bool,
    pub log_level: String,
    /// Playback volume 0.0–1.0.
    #[serde(default = "default_volume")]
    pub volume: f32,
    /// `off`, `all`, or `one`.
    #[serde(default = "default_repeat_str")]
    pub repeat: String,
    #[serde(default)]
    pub shuffle: bool,
    #[serde(default = "default_neon")]
    pub neon: bool,
    /// `standard`, `inverted`, or `mirrored` (spectrogram sub-mode).
    #[serde(default = "default_spectrogram_mode_str")]
    pub spectrogram_mode: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ThemeSettings {
    pub active: String,
    #[serde(default)]
    pub custom: Option<HashMap<String, Theme>>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FetcherSettings {
    pub ytdlp_path: String,
    pub ffmpeg_path: String,
    pub default_format: String,
    pub max_concurrent: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioSettings {
    pub fft_window_size: u32,
    pub fft_hop_size: u32,
    pub fft_rate_hz: u32,
    pub ring_buffer_size: u32,
    /// Number of PCM samples forwarded per FFT frame to the renderers (oscilloscope, vectorscope).
    /// Lower = less memory bandwidth. Default: 1024 (~23ms at 44.1 kHz).
    #[serde(default = "default_pcm_snapshot_samples")]
    pub pcm_snapshot_samples: u32,
    /// Depth of the spectrogram waterfall history ring (rows). Default: 64.
    #[serde(default = "default_spectrogram_history_rows")]
    pub spectrogram_history_rows: u32,
}

/// Performance / quality knobs for the visualizer renderers.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VisualizerSettings {
    /// Number of particle physics sub-steps per frame. Lower = faster. Default: 1.
    #[serde(default = "default_particles_substeps")]
    pub particles_substeps: u32,
    /// Maximum live particle count. Lower = faster. Default: 250.
    #[serde(default = "default_particles_max")]
    pub particles_max: u32,
}

impl Default for VisualizerSettings {
    fn default() -> Self {
        Self {
            particles_substeps: default_particles_substeps(),
            particles_max: default_particles_max(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RtunesConfig {
    pub app: AppSettings,
    pub theme: ThemeSettings,
    pub fetcher: FetcherSettings,
    pub audio: AudioSettings,
    /// Renderer-level performance / quality tuning. All fields have conservative defaults.
    #[serde(default)]
    pub visualizer: VisualizerSettings,
}

/// Load config from `path`, or create it from embedded defaults if missing.
pub fn load_or_create(path: &Path) -> Result<RtunesConfig> {
    if !path.exists() {
        let default: RtunesConfig = serde_yaml::from_str(DEFAULT_CONFIG_YAML).map_err(|e| {
            RtunesError::Config(format!("parse embedded defaults: {e}"))
        })?;
        save(path, &default)?;
        return Ok(default);
    }
    let s = fs::read_to_string(path).map_err(|e| {
        RtunesError::Config(format!("read {}: {e}", path.display()))
    })?;
    serde_yaml::from_str(&s).map_err(|e| {
        RtunesError::Config(format!("{}: {e}", path.display()))
    })
}

/// Write config to `path` (truncate + write + sync).
///
/// Uses a direct write instxead of rename-from-temp so saves succeed reliably on Windows
/// (rename/replace in `%TEMP%` intermittently failed with `os error 2` in CI-style runs).
pub fn save(path: &Path, cfg: &RtunesConfig) -> Result<()> {
    let yaml = serde_yaml::to_string(cfg)
        .map_err(|e| RtunesError::Config(format!("serialize config: {e}")))?;
    let parent = path.parent().ok_or_else(|| {
        RtunesError::Config("config path has no parent directory".into())
    })?;
    fs::create_dir_all(parent)?;
    let mut f = fs::File::create(path).map_err(|e| {
        RtunesError::Config(format!(
            "Failed to save config to {}: {e}. Check write permissions and free disk space.",
            path.display()
        ))
    })?;
    f.write_all(yaml.as_bytes()).map_err(|e| {
        RtunesError::Config(format!("Failed to write {}: {e}", path.display()))
    })?;
    f.sync_all().map_err(|e| {
        RtunesError::Config(format!("Failed to sync {}: {e}", path.display()))
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_invalid_yaml_returns_error_not_panic() {
        let dir = std::env::temp_dir().join(format!("rtunes-bad-yaml-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("config.yaml");
        std::fs::write(&path, "version: [unterminated").expect("write");
        let err = load_or_create(&path).expect_err("invalid yaml must error");
        let msg = err.to_string();
        assert!(
            msg.contains("config.yaml") || msg.contains("parse"),
            "unexpected: {msg}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_round_trip() {
        let dir = std::env::temp_dir().join(format!("rtunes-cfg-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("config.yaml");
        let mut cfg = load_or_create(&path).expect("load_or_create");
        cfg.app.fps = 42;
        save(&path, &cfg).expect("save");
        let again = load_or_create(&path).expect("reload");
        assert_eq!(again.app.fps, 42);
        assert_eq!(again.theme.active, cfg.theme.active);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn legacy_config_loads_with_defaults_for_new_fields() {
        let dir = std::env::temp_dir().join(format!("rtunes-legacy-cfg-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).expect("mkdir");
        let path = dir.join("config.yaml");
        let yaml = r#"app:
  library_paths: []
  download_dir: "~"
  fps: 60
  default_visualizer: spectrum
  start_fullscreen: false
  log_level: warn
theme:
  active: synthwave
fetcher:
  ytdlp_path: auto
  ffmpeg_path: auto
  default_format: mp3
  max_concurrent: 3
audio:
  fft_window_size: 4096
  fft_hop_size: 2048
  fft_rate_hz: 30
  ring_buffer_size: 16384
"#;
        std::fs::write(&path, yaml).expect("write");
        let cfg = load_or_create(&path).expect("load");
        assert!((cfg.app.volume - 0.7).abs() < f32::EPSILON);
        assert_eq!(cfg.app.repeat, "off");
        assert!(!cfg.app.shuffle);
        assert!(cfg.app.neon);
        assert_eq!(cfg.app.spectrogram_mode, "standard");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn config_path_default_is_config_yaml_beside_exe() {
        if std::env::var(CONFIG_ENV).is_ok() {
            return;
        }
        let p = config_path();
        assert_eq!(p.file_name().and_then(|s| s.to_str()), Some("config.yaml"));
    }
}
