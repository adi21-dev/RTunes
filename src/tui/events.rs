//! Crossterm keyboard dispatch into [`crate::app::state::AppState`].

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use crossterm::event::{Event, KeyCode, KeyEventKind, KeyModifiers};

use crate::app::lock_shared;
use crate::app::state::{
    AppState, InputMode, LibraryFolder, PanelFocus, RepeatMode, SettingsRow, SpectrogramMode,
    VisualizerMode,
};
use crate::cli::resolve_library_roots;
use crate::config::theme::normalize_theme_key;
use crate::config::{resolve_active_theme, FetcherSettings, RtunesConfig, Theme};
use crate::fetcher::{
    deps_dir, download_ffmpeg, download_ytdlp, ffmpeg_auto_download_supported,
    ffmpeg_manual_instructions, try_resolve_tools, validate_url, FetchEvent, FetchOpts,
    FetcherPool, MissingTool, PickerEvent, PickerTarget,
};
use crate::library::{is_scanning, scan_async, ScanEvent};

const BUILTIN_THEME_ORDER: &[&str] = &["synthwave", "dracula", "nord", "tokyo_night", "monochrome"];

const VIZ_ORDER: [VisualizerMode; 8] = [
    VisualizerMode::Spectrum,
    VisualizerMode::Spectrogram,
    VisualizerMode::Oscilloscope,
    VisualizerMode::Vectorscope,
    VisualizerMode::Supernova,
    VisualizerMode::PulseRings,
    VisualizerMode::BandMeter,
    VisualizerMode::Particles,
];

/// Shared handles for the TUI event loop (config + download pool).
pub struct TuiDeps {
    pub config: Arc<Mutex<RtunesConfig>>,
    pub config_path: PathBuf,
    pub fetch_pool: Arc<FetcherPool>,
    pub fetch_tx: crossbeam_channel::Sender<FetchEvent>,
    /// Channel for delivering native picker results back to the event loop.
    pub picker_tx: crossbeam_channel::Sender<PickerEvent>,
    /// Shared fetcher settings — kept in sync with config so the live YtDlpFetcher
    /// reads updated paths without a restart.
    pub fetcher_settings: Arc<Mutex<FetcherSettings>>,
}

fn canonical_for_config_entry(s: &str) -> Option<PathBuf> {
    let exp = crate::utils::expand_path(s);
    dunce::canonicalize(&exp).ok()
}

fn download_output_dir(cfg: &RtunesConfig) -> PathBuf {
    let roots = resolve_library_roots(&cfg.app.library_paths);
    if let Some(p) = roots.first() {
        return p.clone();
    }
    crate::utils::expand_path(&cfg.app.download_dir)
}

fn persist_cfg(deps: &TuiDeps, mutate: impl FnOnce(&mut RtunesConfig)) {
    let snapshot = {
        let mut c = lock_shared(&deps.config);
        mutate(&mut c);
        c.clone()
    };
    if let Err(e) = crate::config::save(&deps.config_path, &snapshot) {
        tracing::warn!(error = %e, "failed to save config");
    }
}

pub fn sync_library_folders_from_config(app: &mut AppState, cfg: &RtunesConfig) {
    app.library_folders = cfg
        .app
        .library_paths
        .iter()
        .map(|p| LibraryFolder {
            path: crate::utils::expand_path(p),
            track_count: 0,
            last_scanned: None,
        })
        .collect();
    if app.selected_folder >= app.library_folders.len() {
        app.selected_folder = app.library_folders.len().saturating_sub(1);
    }
}

fn set_toast(app: &mut AppState, msg: impl Into<String>) {
    app.message = Some((msg.into(), Instant::now()));
}

/// Live filter while typing in Search mode.
pub fn recompute_filter(app: &mut AppState) {
    let q = app.input_buffer.to_lowercase();
    if q.is_empty() {
        app.filtered_indices.clear();
        if app.selected_track > 0 && app.library.is_empty() {
            app.selected_track = 0;
        }
        return;
    }
    app.filtered_indices = app
        .library
        .iter()
        .enumerate()
        .filter(|(_, t)| {
            let hay = format!(
                "{} {} {}",
                t.title,
                t.artist.as_deref().unwrap_or(""),
                t.album.as_deref().unwrap_or("")
            )
            .to_lowercase();
            hay.contains(&q)
        })
        .map(|(i, _)| i)
        .collect();
    let n = app.filtered_indices.len();
    if n == 0 {
        app.selected_track = 0;
    } else if app.selected_track >= n {
        app.selected_track = n - 1;
    }
}

fn visible_track_indices(app: &AppState) -> Vec<usize> {
    if !app.filtered_indices.is_empty() {
        app.filtered_indices.clone()
    } else {
        (0..app.library.len()).collect()
    }
}

fn visible_len(app: &AppState) -> usize {
    visible_track_indices(app).len()
}

pub fn seek_relative(app: &mut AppState, delta_secs: f64) {
    let dur = app.player.duration_secs.max(0.0);
    let pos = app.player.position_secs;
    let next = (pos + delta_secs).clamp(0.0, dur);
    app.player.seek_to = Some(next);
}

pub fn bump_volume(app: &mut AppState, delta: f32) {
    app.player.volume = (app.player.volume + delta).clamp(0.0, 1.0);
}

pub fn cycle_repeat(app: &mut AppState) {
    app.player.repeat = match app.player.repeat {
        RepeatMode::Off => RepeatMode::All,
        RepeatMode::All => RepeatMode::One,
        RepeatMode::One => RepeatMode::Off,
    };
    let label = match app.player.repeat {
        RepeatMode::Off => "off",
        RepeatMode::All => "all",
        RepeatMode::One => "one",
    };
    set_toast(app, format!("Repeat: {label}"));
}

fn cycle_visualizer(app: &mut AppState, backward: bool) {
    let i = VIZ_ORDER
        .iter()
        .position(|&v| v == app.visualizer_mode)
        .unwrap_or(0);
    let n = VIZ_ORDER.len();
    let j = if backward {
        (i + n - 1) % n
    } else {
        (i + 1) % n
    };
    app.visualizer_mode = VIZ_ORDER[j];
}

fn visualizer_from_digit(c: char) -> Option<VisualizerMode> {
    if !matches!(c, '1'..='9') {
        return None;
    }
    let idx = (c as u8 - b'1') as usize;
    VIZ_ORDER.get(idx).copied()
}

/// Returns the persisted theme key (e.g. `synthwave`).
fn cycle_theme(theme: &Arc<Mutex<Theme>>, custom: Option<&HashMap<String, Theme>>) -> String {
    let mut g = lock_shared(theme);
    let key = normalize_theme_key(&g.name);
    let pos = BUILTIN_THEME_ORDER
        .iter()
        .position(|&k| k == key.as_str())
        .unwrap_or(0);
    let next = BUILTIN_THEME_ORDER[(pos + 1) % BUILTIN_THEME_ORDER.len()];
    let t = resolve_active_theme(next, custom);
    let name = t.name.clone();
    *g = t;
    drop(g);
    tracing::info!(theme = %name, "theme cycled");
    next.to_string()
}

fn cycle_theme_with_toast(
    state: &Arc<Mutex<AppState>>,
    theme: &Arc<Mutex<Theme>>,
    custom: Option<&HashMap<String, Theme>>,
    deps: &TuiDeps,
) {
    let active_key = cycle_theme(theme, custom);
    persist_cfg(deps, |c| {
        c.theme.active = active_key;
    });
    let name = lock_shared(theme).name.clone();
    let mut g = lock_shared(state);
    set_toast(&mut g, format!("Theme: {name}"));
}

fn next_track(app: &mut AppState) {
    if app.library.is_empty() {
        return;
    }
    let vis = visible_track_indices(app);
    if vis.is_empty() {
        return;
    }
    let cur_lib = app.player.current_index;
    let pos_in_vis = vis
        .iter()
        .position(|&i| Some(i) == cur_lib)
        .unwrap_or(app.selected_track);
    let next_pos = (pos_in_vis + 1) % vis.len();
    let idx = vis[next_pos];
    app.player.current_index = Some(idx);
    app.selected_track = next_pos;
    app.player.position_secs = 0.0;
    app.player.seek_to = None;
}

fn prev_track(app: &mut AppState) {
    if app.library.is_empty() {
        return;
    }
    let vis = visible_track_indices(app);
    if vis.is_empty() {
        return;
    }
    let cur_lib = app.player.current_index;
    let pos_in_vis = vis
        .iter()
        .position(|&i| Some(i) == cur_lib)
        .unwrap_or(app.selected_track);
    let next_pos = if pos_in_vis == 0 {
        if app.player.repeat == RepeatMode::All {
            vis.len() - 1
        } else {
            0
        }
    } else {
        pos_in_vis - 1
    };
    let idx = vis[next_pos];
    app.player.current_index = Some(idx);
    app.selected_track = next_pos;
    app.player.position_secs = 0.0;
    app.player.seek_to = None;
}

fn enter_play_selected(app: &mut AppState) {
    let vis = visible_track_indices(app);
    if vis.is_empty() {
        return;
    }
    let st = app.selected_track.min(vis.len() - 1);
    let idx = vis[st];
    if let Some(t) = app.library.get(idx) {
        app.player.current_index = Some(idx);
        app.player.duration_secs = t.duration_secs as f64;
        app.player.position_secs = 0.0;
        app.player.seek_to = None;
        app.player.is_playing = true;
    }
}

fn recompute_folder_counts(app: &mut AppState) {
    let now = Instant::now();
    for folder in &mut app.library_folders {
        let p = folder.path.as_path();
        folder.track_count = app
            .library
            .iter()
            .filter(|t| t.filepath.starts_with(p))
            .count();
        folder.last_scanned = Some(now);
    }
}

/// Start background rescan; updates library + toast when complete.
pub fn trigger_rescan(state: &Arc<Mutex<AppState>>, config: &Arc<Mutex<RtunesConfig>>) {
    let paths = lock_shared(config).app.library_paths.clone();
    if is_scanning() {
        let mut g = lock_shared(state);
        g.rescan_pending = true;
        set_toast(
            &mut g,
            "Rescan already in progress; will rescan when complete.",
        );
        return;
    }
    let roots = resolve_library_roots(&paths);
    if roots.is_empty() {
        let mut g = lock_shared(state);
        set_toast(&mut g, "No library folders to scan.");
        return;
    }
    let cur_id = {
        let g = lock_shared(state);
        g.player
            .current_index
            .and_then(|i| g.library.get(i).map(|t| t.id.clone()))
    };
    let (tx, rx) = crossbeam_channel::unbounded();
    let Some(scan_handle) = scan_async(roots, tx) else {
        let mut g = lock_shared(state);
        set_toast(
            &mut g,
            "Rescan already in progress; will rescan when complete.",
        );
        return;
    };
    {
        let mut g = lock_shared(state);
        g.is_rescanning = true;
    }
    let state2 = state.clone();
    std::thread::spawn(move || {
        while let Ok(ev) = rx.recv() {
            if let ScanEvent::Done(tracks) = ev {
                let mut g = lock_shared(&state2);
                let n = tracks.len();
                g.library = tracks;
                recompute_folder_counts(&mut g);
                if let Some(id) = cur_id {
                    if let Some((new_idx, _)) =
                        g.library.iter().enumerate().find(|(_, t)| t.id == id)
                    {
                        g.player.current_index = Some(new_idx);
                    } else {
                        g.player.current_index = None;
                    }
                }
                g.is_rescanning = false;
                g.message = Some((
                    format!("Rescan complete. Found {n} tracks."),
                    Instant::now(),
                ));
                break;
            }
        }
        let _ = scan_handle.join();
    });
}

/// Handle a completed picker result: persist to config and update app state.
pub fn handle_picker_event(state: &Arc<Mutex<AppState>>, deps: &TuiDeps, ev: PickerEvent) {
    let PickerEvent { target, path } = ev;
    let Some(path) = path else {
        // User cancelled the dialog — no-op.
        return;
    };
    let path_str = path.to_string_lossy().into_owned();
    let label = match target {
        PickerTarget::YtDlp => "yt-dlp",
        PickerTarget::Ffmpeg => "ffmpeg",
        PickerTarget::DownloadDir => "Download dir",
    };
    {
        let mut c = lock_shared(&deps.config);
        match target {
            PickerTarget::YtDlp => c.fetcher.ytdlp_path = path_str.clone(),
            PickerTarget::Ffmpeg => c.fetcher.ffmpeg_path = path_str.clone(),
            PickerTarget::DownloadDir => c.app.download_dir = path_str.clone(),
        }
    }
    // Propagate yt-dlp/ffmpeg path changes to the live fetcher immediately.
    {
        let mut fs = lock_shared(&deps.fetcher_settings);
        match target {
            PickerTarget::YtDlp => fs.ytdlp_path = path_str.clone(),
            PickerTarget::Ffmpeg => fs.ffmpeg_path = path_str.clone(),
            PickerTarget::DownloadDir => {}
        }
    }
    let saved = lock_shared(&deps.config).clone();
    if let Err(e) = crate::config::save(&deps.config_path, &saved) {
        tracing::warn!(error = %e, "failed to save config after picker");
    }
    let mut g = lock_shared(state);
    match target {
        PickerTarget::YtDlp => g.settings_ytdlp_value = path_str.clone(),
        PickerTarget::Ffmpeg => g.settings_ffmpeg_value = path_str.clone(),
        PickerTarget::DownloadDir => g.settings_download_dir = path_str.clone(),
    }
    set_toast(&mut g, format!("{label} path saved"));
}

fn handle_download_enter(
    state: &Arc<Mutex<AppState>>,
    deps: &TuiDeps,
    buf: &str,
) -> anyhow::Result<()> {
    match validate_url(buf.trim()) {
        Err(e) => {
            let mut g = lock_shared(state);
            set_toast(&mut g, e.to_string());
        }
        Ok(url) => {
            let (fmt, out_dir) = {
                let c = lock_shared(&deps.config);
                (c.fetcher.default_format.clone(), download_output_dir(&c))
            };
            let fetch_opts = FetchOpts {
                format: fmt,
                output_dir: out_dir,
            };
            // Pre-flight: check whether yt-dlp and ffmpeg are available.
            let fetcher_settings = lock_shared(&deps.fetcher_settings).clone();
            if let Err(missing) = try_resolve_tools(&fetcher_settings) {
                // Store the pending fetch and show a consent prompt instead of failing.
                let mut g = lock_shared(state);
                g.pending_fetch = Some((url, fetch_opts));
                g.deps_prompt = Some(missing);
                set_toast(
                    &mut g,
                    "yt-dlp/ffmpeg not found. Press Y to auto-download (~120MB) or N to cancel.",
                );
            } else {
                {
                    let mut g = lock_shared(state);
                    g.download_progress = Some(0.0);
                    g.download_stage = Some("Starting\u{2026}".into());
                }
                deps.fetch_pool
                    .submit(url, fetch_opts, deps.fetch_tx.clone());
            }
        }
    }
    Ok(())
}

fn handle_add_folder_enter(
    state: &Arc<Mutex<AppState>>,
    deps: &TuiDeps,
    buf: &str,
) -> anyhow::Result<()> {
    let exp = crate::utils::expand_path(buf.trim());
    let md = match std::fs::metadata(&exp) {
        Ok(m) => m,
        Err(_) => {
            let mut g = lock_shared(state);
            set_toast(&mut g, format!("Not a directory: {}", exp.display()));
            return Ok(());
        }
    };
    if !md.is_dir() {
        let mut g = lock_shared(state);
        set_toast(&mut g, format!("Not a directory: {}", exp.display()));
        return Ok(());
    }
    let canon = match dunce::canonicalize(&exp) {
        Ok(c) => c,
        Err(e) => {
            let mut g = lock_shared(state);
            set_toast(&mut g, format!("{}", e));
            return Ok(());
        }
    };
    {
        let mut c = lock_shared(&deps.config);
        for p in &c.app.library_paths {
            if canonical_for_config_entry(p).as_ref() == Some(&canon) {
                let mut g = lock_shared(state);
                set_toast(&mut g, "Folder already in library.");
                return Ok(());
            }
        }
        c.app
            .library_paths
            .push(canon.to_string_lossy().into_owned());
        let saved = c.clone();
        drop(c);
        crate::config::save(&deps.config_path, &saved)
            .map_err(|e| anyhow::anyhow!(e.to_string()))?;
    }
    let cfg_snapshot = lock_shared(&deps.config).clone();
    {
        let mut g = lock_shared(state);
        sync_library_folders_from_config(&mut g, &cfg_snapshot);
        set_toast(&mut g, format!("Added: {}", canon.display()));
    }
    trigger_rescan(state, &deps.config);
    Ok(())
}

fn handle_remove_folder(
    state: &Arc<Mutex<AppState>>,
    deps: &TuiDeps,
    folder_path: &Path,
) -> anyhow::Result<()> {
    let target = dunce::canonicalize(folder_path).unwrap_or_else(|_| folder_path.to_path_buf());
    let removed = {
        let mut c = lock_shared(&deps.config);
        let before = c.app.library_paths.len();
        c.app
            .library_paths
            .retain(|s| canonical_for_config_entry(s).as_ref() != Some(&target));
        c.app.library_paths.len() < before
    };
    if !removed {
        let mut g = lock_shared(state);
        set_toast(&mut g, "Could not remove folder from config.");
        return Ok(());
    }
    let saved = lock_shared(&deps.config).clone();
    crate::config::save(&deps.config_path, &saved).map_err(|e| anyhow::anyhow!(e.to_string()))?;
    let cfg_snapshot = lock_shared(&deps.config).clone();
    {
        let mut g = lock_shared(state);
        sync_library_folders_from_config(&mut g, &cfg_snapshot);
        if g.library_folders.is_empty() {
            set_toast(
                &mut g,
                "All library folders removed. Press 'a' to add a new one.",
            );
        } else {
            set_toast(&mut g, format!("Removed: {}", folder_path.display()));
        }
    }
    trigger_rescan(state, &deps.config);
    Ok(())
}

pub fn handle_event(
    state: &Arc<Mutex<AppState>>,
    theme: &Arc<Mutex<Theme>>,
    custom_themes: Option<&HashMap<String, Theme>>,
    deps: &TuiDeps,
    ev: &Event,
) -> anyhow::Result<()> {
    match ev {
        Event::Resize(w, h) => {
            tracing::debug!(w, h, "terminal resize");
        }
        Event::Key(key) => {
            if key.kind != KeyEventKind::Press {
                return Ok(());
            }
            dispatch_key(state, theme, custom_themes, deps, key)?;
        }
        _ => {}
    }
    Ok(())
}

fn dispatch_key(
    state: &Arc<Mutex<AppState>>,
    theme: &Arc<Mutex<Theme>>,
    custom_themes: Option<&HashMap<String, Theme>>,
    deps: &TuiDeps,
    key: &crossterm::event::KeyEvent,
) -> anyhow::Result<()> {
    let code = key.code;
    let mods = key.modifiers;
    let shift = mods.contains(KeyModifiers::SHIFT);

    let mut app = lock_shared(state);
    let mode = app.input_mode;

    // Global quit
    if matches!(mode, InputMode::Normal)
        && matches!(code, KeyCode::Char('q'))
        && !mods.contains(KeyModifiers::CONTROL)
    {
        app.quit = true;
        return Ok(());
    }

    // F2 opens the Settings overlay from any mode.
    if matches!(code, KeyCode::F(2)) {
        let (ytdlp, ffmpeg, dl_dir) = {
            let c = lock_shared(&deps.config);
            (
                c.fetcher.ytdlp_path.clone(),
                c.fetcher.ffmpeg_path.clone(),
                c.app.download_dir.clone(),
            )
        };
        app.show_settings = true;
        app.settings_ytdlp_value = ytdlp;
        app.settings_ffmpeg_value = ffmpeg;
        app.settings_download_dir = dl_dir;
        app.settings_row = SettingsRow::default();
        app.input_mode = InputMode::Settings;
        return Ok(());
    }

    match mode {
        InputMode::Settings => match code {
            KeyCode::Esc => {
                app.show_settings = false;
                app.input_mode = InputMode::Normal;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.settings_row = match app.settings_row {
                    SettingsRow::YtDlp => SettingsRow::YtDlp,
                    SettingsRow::Ffmpeg => SettingsRow::YtDlp,
                    SettingsRow::DownloadDir => SettingsRow::Ffmpeg,
                };
            }
            KeyCode::Down | KeyCode::Char('j') => {
                app.settings_row = match app.settings_row {
                    SettingsRow::YtDlp => SettingsRow::Ffmpeg,
                    SettingsRow::Ffmpeg => SettingsRow::DownloadDir,
                    SettingsRow::DownloadDir => SettingsRow::DownloadDir,
                };
            }
            KeyCode::Char('r') => {
                let row = app.settings_row;
                match row {
                    SettingsRow::YtDlp => {
                        app.settings_ytdlp_value = "auto".into();
                    }
                    SettingsRow::Ffmpeg => {
                        app.settings_ffmpeg_value = "auto".into();
                    }
                    SettingsRow::DownloadDir => {
                        app.settings_download_dir = "~".into();
                    }
                }
                drop(app);
                persist_cfg(deps, |c| match row {
                    SettingsRow::YtDlp => c.fetcher.ytdlp_path = "auto".into(),
                    SettingsRow::Ffmpeg => c.fetcher.ffmpeg_path = "auto".into(),
                    SettingsRow::DownloadDir => c.app.download_dir = "~".into(),
                });
                let label = match row {
                    SettingsRow::YtDlp => "yt-dlp",
                    SettingsRow::Ffmpeg => "ffmpeg",
                    SettingsRow::DownloadDir => "Download dir",
                };
                set_toast(&mut lock_shared(state), format!("{label} path reset"));
                return Ok(());
            }
            KeyCode::Enter | KeyCode::Char('d') => {
                let row = app.settings_row;
                let target = match row {
                    SettingsRow::YtDlp => PickerTarget::YtDlp,
                    SettingsRow::Ffmpeg => PickerTarget::Ffmpeg,
                    SettingsRow::DownloadDir => PickerTarget::DownloadDir,
                };
                // DownloadDir always opens a directory picker.
                let open_dir =
                    matches!(code, KeyCode::Char('d')) || matches!(row, SettingsRow::DownloadDir);
                let current_val = match row {
                    SettingsRow::YtDlp => app.settings_ytdlp_value.clone(),
                    SettingsRow::Ffmpeg => app.settings_ffmpeg_value.clone(),
                    SettingsRow::DownloadDir => app.settings_download_dir.clone(),
                };
                drop(app);
                let suggested = {
                    let p = crate::utils::expand_path(&current_val);
                    if p.is_file() {
                        p.parent().map(|p| p.to_path_buf())
                    } else if p.is_dir() {
                        Some(p)
                    } else {
                        None
                    }
                };
                if open_dir {
                    crate::fetcher::open_dir_picker_async(
                        target,
                        suggested,
                        deps.picker_tx.clone(),
                    );
                } else {
                    crate::fetcher::open_binary_picker_async(
                        target,
                        suggested,
                        deps.picker_tx.clone(),
                    );
                }
                return Ok(());
            }
            _ => {}
        },
        InputMode::Search => match code {
            KeyCode::Esc => {
                app.input_buffer.clear();
                app.search_query = None;
                app.filtered_indices.clear();
                app.input_mode = InputMode::Normal;
            }
            KeyCode::Enter => {
                app.search_query = Some(app.input_buffer.clone());
                app.input_mode = InputMode::Normal;
            }
            KeyCode::Backspace => {
                app.input_buffer.pop();
                recompute_filter(&mut app);
            }
            KeyCode::Char(c) => {
                app.input_buffer.push(c);
                recompute_filter(&mut app);
            }
            _ => {}
        },
        InputMode::DownloadUrl | InputMode::AddLibraryPath => match code {
            KeyCode::Esc => {
                app.input_buffer.clear();
                app.input_mode = if app.show_library_manager {
                    InputMode::LibraryManager
                } else {
                    InputMode::Normal
                };
            }
            KeyCode::Enter => {
                let is_download = matches!(mode, InputMode::DownloadUrl);
                let buf = app.input_buffer.clone();
                app.input_buffer.clear();
                app.input_mode = if app.show_library_manager {
                    InputMode::LibraryManager
                } else {
                    InputMode::Normal
                };
                drop(app);
                if is_download {
                    handle_download_enter(state, deps, &buf)?;
                } else {
                    handle_add_folder_enter(state, deps, &buf)?;
                }
                return Ok(());
            }
            KeyCode::Backspace => {
                app.input_buffer.pop();
            }
            KeyCode::Char(c) => {
                app.input_buffer.push(c);
            }
            _ => {}
        },
        InputMode::LibraryManager => match code {
            KeyCode::Esc => {
                app.show_library_manager = false;
                app.input_mode = InputMode::Normal;
            }
            KeyCode::Up | KeyCode::Char('k') => {
                app.selected_folder = app.selected_folder.saturating_sub(1);
            }
            KeyCode::Down | KeyCode::Char('j') => {
                let max = app.library_folders.len().saturating_sub(1);
                app.selected_folder = (app.selected_folder + 1).min(max);
            }
            KeyCode::Char('a') => {
                app.input_mode = InputMode::AddLibraryPath;
                app.input_buffer.clear();
            }
            KeyCode::Char('x') | KeyCode::Delete => {
                let folder = app
                    .library_folders
                    .get(app.selected_folder)
                    .map(|f| f.path.clone());
                if let Some(path) = folder {
                    drop(app);
                    handle_remove_folder(state, deps, &path)?;
                } else {
                    set_toast(&mut app, "No folder selected.");
                }
                return Ok(());
            }
            KeyCode::Char('R') => {
                drop(app);
                trigger_rescan(state, &deps.config);
                return Ok(());
            }
            _ => {}
        },
        InputMode::Normal => {
            // Handle deps consent prompt (Y/N) before anything else.
            if app.deps_prompt.is_some() {
                match code {
                    KeyCode::Char('y') | KeyCode::Char('Y') => {
                        let missing = app.deps_prompt.take().unwrap_or_default();
                        let pending = app.pending_fetch.take();
                        app.download_progress = Some(0.0);
                        app.download_stage = Some("Preparing dep download…".into());
                        drop(app);

                        // Spawn a thread that downloads the missing deps and sends
                        // DepsDownloading progress events, then DepsReady or Failed.
                        let tx = deps.fetch_tx.clone();
                        let pool = deps.fetch_pool.clone();
                        let fetcher_settings_arc = deps.fetcher_settings.clone();
                        let state2 = state.clone();
                        std::thread::spawn(move || {
                            let dd = match deps_dir() {
                                Some(d) => d,
                                None => {
                                    let _ = tx.send(FetchEvent::Failed(
                                        "Could not determine deps/ directory".into(),
                                    ));
                                    return;
                                }
                            };

                            let needs_ytdlp = missing.contains(&MissingTool::YtDlp);
                            let needs_ffmpeg = missing.contains(&MissingTool::Ffmpeg);

                            if needs_ytdlp {
                                let tx2 = tx.clone();
                                let res = download_ytdlp(&dd, |p| {
                                    let _ = tx2.send(FetchEvent::DepsDownloading {
                                        tool: "yt-dlp".into(),
                                        progress: p,
                                    });
                                });
                                if let Err(e) = res {
                                    let _ = tx.send(FetchEvent::Failed(format!(
                                        "yt-dlp download failed: {e}"
                                    )));
                                    return;
                                }
                            }

                            if needs_ffmpeg {
                                if !ffmpeg_auto_download_supported() {
                                    let _ = tx.send(FetchEvent::Failed(format!(
                                        "ffmpeg not found. {}",
                                        ffmpeg_manual_instructions()
                                    )));
                                    return;
                                }
                                let tx2 = tx.clone();
                                let res = download_ffmpeg(&dd, |p| {
                                    let _ = tx2.send(FetchEvent::DepsDownloading {
                                        tool: "ffmpeg".into(),
                                        progress: p,
                                    });
                                });
                                if let Err(e) = res {
                                    let _ = tx.send(FetchEvent::Failed(format!(
                                        "ffmpeg download failed: {e}"
                                    )));
                                    return;
                                }
                            }

                            // Update shared fetcher settings so the live fetcher sees the new paths.
                            {
                                let mut fs = fetcher_settings_arc
                                    .lock()
                                    .unwrap_or_else(|p| p.into_inner());
                                if needs_ytdlp {
                                    fs.ytdlp_path = "auto".into();
                                }
                                if needs_ffmpeg {
                                    fs.ffmpeg_path = "auto".into();
                                }
                            }
                            {
                                let mut g = state2.lock().unwrap_or_else(|p| p.into_inner());
                                g.download_progress = Some(0.0);
                                g.download_stage = Some("Starting\u{2026}".into());
                            }

                            let _ = tx.send(FetchEvent::DepsReady);

                            // Retry the pending fetch if there is one.
                            if let Some((url, opts)) = pending {
                                pool.submit(url, opts, tx);
                            }
                        });
                        return Ok(());
                    }
                    KeyCode::Char('n') | KeyCode::Char('N') | KeyCode::Esc => {
                        app.deps_prompt = None;
                        app.pending_fetch = None;
                        set_toast(
                            &mut app,
                            "Download cancelled. Install yt-dlp/ffmpeg or use F2 to set paths.",
                        );
                        return Ok(());
                    }
                    _ => {
                        // Swallow other keys while prompt is active.
                        return Ok(());
                    }
                }
            }

            if app.show_help && matches!(code, KeyCode::Esc | KeyCode::Char('?')) {
                app.show_help = false;
                return Ok(());
            }
            if matches!(code, KeyCode::Esc) {
                if app.show_help {
                    app.show_help = false;
                } else if app.show_library_manager {
                    app.show_library_manager = false;
                } else {
                    app.quit = true;
                }
                return Ok(());
            }

            match code {
                KeyCode::Tab => {
                    if app.show_help || app.show_library_manager {
                        // Let overlays keep focus; Tab does nothing.
                    } else if app.is_fullscreen {
                        app.is_fullscreen = false;
                        app.panel_focus = PanelFocus::Normal;
                    } else {
                        match app.panel_focus {
                            PanelFocus::Normal => app.panel_focus = PanelFocus::TransportOnly,
                            PanelFocus::TransportOnly => app.is_fullscreen = true,
                        }
                    }
                }
                KeyCode::Char('?') => {
                    app.show_help = !app.show_help;
                }
                KeyCode::Char(' ') => {
                    app.player.is_playing = !app.player.is_playing;
                }
                KeyCode::Char('n') => {
                    next_track(&mut app);
                }
                KeyCode::Char('p') => {
                    prev_track(&mut app);
                }
                KeyCode::Right => {
                    seek_relative(&mut app, if shift { 30.0 } else { 5.0 });
                }
                KeyCode::Left => {
                    seek_relative(&mut app, if shift { -30.0 } else { -5.0 });
                }
                KeyCode::Char('l') => {
                    if mods.contains(KeyModifiers::CONTROL) {
                        app.show_library_manager = true;
                        app.input_mode = InputMode::LibraryManager;
                    } else {
                        seek_relative(&mut app, if shift { 30.0 } else { 5.0 });
                    }
                }
                KeyCode::Char('L') => {
                    if mods.contains(KeyModifiers::CONTROL) {
                        app.show_library_manager = true;
                        app.input_mode = InputMode::LibraryManager;
                    } else {
                        seek_relative(&mut app, 30.0);
                    }
                }
                KeyCode::Char('h') => {
                    seek_relative(&mut app, if shift { -30.0 } else { -5.0 });
                }
                KeyCode::Char('H') => {
                    seek_relative(&mut app, -30.0);
                }
                KeyCode::Char('+') | KeyCode::Char('=') => {
                    bump_volume(&mut app, 0.05);
                    let v = app.player.volume;
                    drop(app);
                    persist_cfg(deps, |c| {
                        c.app.volume = v;
                    });
                    return Ok(());
                }
                KeyCode::Char('-') => {
                    bump_volume(&mut app, -0.05);
                    let v = app.player.volume;
                    drop(app);
                    persist_cfg(deps, |c| {
                        c.app.volume = v;
                    });
                    return Ok(());
                }
                KeyCode::Char('M')
                    if shift && app.visualizer_mode == VisualizerMode::Spectrogram =>
                {
                    app.spectrogram_mode = match app.spectrogram_mode {
                        SpectrogramMode::Standard => SpectrogramMode::Inverted,
                        SpectrogramMode::Inverted => SpectrogramMode::Mirrored,
                        SpectrogramMode::Mirrored => SpectrogramMode::Standard,
                    };
                    let m = app.spectrogram_mode;
                    set_toast(&mut app, format!("Spectrogram: {:?}", m));
                    let spec_str = m.as_config_str().to_string();
                    drop(app);
                    persist_cfg(deps, |c| {
                        c.app.spectrogram_mode = spec_str;
                    });
                    return Ok(());
                }
                KeyCode::Char('m') => {
                    app.player.muted = !app.player.muted;
                }
                KeyCode::Char('s') => {
                    app.player.shuffle = !app.player.shuffle;
                    let msg = if app.player.shuffle {
                        "Shuffle ON"
                    } else {
                        "Shuffle OFF"
                    };
                    set_toast(&mut app, msg);
                    let sh = app.player.shuffle;
                    drop(app);
                    persist_cfg(deps, |c| {
                        c.app.shuffle = sh;
                    });
                    return Ok(());
                }
                KeyCode::Char('R') => {
                    drop(app);
                    trigger_rescan(state, &deps.config);
                    return Ok(());
                }
                KeyCode::Char('r') => {
                    cycle_repeat(&mut app);
                    let rep = app.player.repeat.as_config_str().to_string();
                    drop(app);
                    persist_cfg(deps, |c| {
                        c.app.repeat = rep;
                    });
                    return Ok(());
                }
                KeyCode::Up | KeyCode::Char('k') => {
                    app.selected_track = app.selected_track.saturating_sub(1);
                }
                KeyCode::Down | KeyCode::Char('j') => {
                    let vl = visible_len(&app);
                    if vl > 0 {
                        app.selected_track = (app.selected_track + 1).min(vl - 1);
                    }
                }
                KeyCode::Enter => {
                    enter_play_selected(&mut app);
                }
                KeyCode::Char('/') => {
                    app.input_mode = InputMode::Search;
                    app.input_buffer.clear();
                    recompute_filter(&mut app);
                }
                KeyCode::Char('v') => {
                    cycle_visualizer(&mut app, shift);
                    let viz = app.visualizer_mode.to_string();
                    drop(app);
                    persist_cfg(deps, |c| {
                        c.app.default_visualizer = viz;
                    });
                    return Ok(());
                }
                KeyCode::Char('V') => {
                    cycle_visualizer(&mut app, true);
                    let viz = app.visualizer_mode.to_string();
                    drop(app);
                    persist_cfg(deps, |c| {
                        c.app.default_visualizer = viz;
                    });
                    return Ok(());
                }
                KeyCode::Char(c) if matches!(c, '1'..='9') => {
                    if let Some(v) = visualizer_from_digit(c) {
                        app.visualizer_mode = v;
                        let viz = app.visualizer_mode.to_string();
                        drop(app);
                        persist_cfg(deps, |c| {
                            c.app.default_visualizer = viz;
                        });
                        return Ok(());
                    }
                }
                KeyCode::Char('t') => {
                    drop(app);
                    cycle_theme_with_toast(state, theme, custom_themes, deps);
                    return Ok(());
                }
                KeyCode::Char('g') => {
                    app.neon_enabled = !app.neon_enabled;
                    let msg = if app.neon_enabled {
                        "Neon ON"
                    } else {
                        "Neon OFF"
                    };
                    set_toast(&mut app, msg);
                    let n = app.neon_enabled;
                    drop(app);
                    persist_cfg(deps, |c| {
                        c.app.neon = n;
                    });
                    return Ok(());
                }
                KeyCode::Char('f') => {
                    app.is_fullscreen = !app.is_fullscreen;
                    if !app.is_fullscreen {
                        app.panel_focus = PanelFocus::Normal;
                    }
                }
                KeyCode::Char('d') => {
                    if mods.contains(KeyModifiers::CONTROL) {
                        // Ctrl+D: request immediate audio device reconnect.
                        app.player.force_reconnect = true;
                        set_toast(&mut app, "Reconnecting audio device…");
                    } else {
                        app.input_mode = InputMode::DownloadUrl;
                        app.input_buffer.clear();
                    }
                }
                KeyCode::Char('a') => {
                    app.input_mode = InputMode::AddLibraryPath;
                    app.input_buffer.clear();
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::{PanelFocus, Track};
    use crate::config::{resolve_active_theme, RtunesConfig};
    use crate::fetcher::MockFetcher;
    use crossterm::event::{KeyCode, KeyEvent, KeyModifiers};
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::sync::Arc;

    static TEST_CFG_COUNTER: AtomicU64 = AtomicU64::new(0);

    fn test_deps(cfg: RtunesConfig) -> TuiDeps {
        let (tx, _rx) = crossbeam_channel::unbounded();
        let (picker_tx, _picker_rx) = crossbeam_channel::unbounded();
        let n = TEST_CFG_COUNTER.fetch_add(1, Ordering::Relaxed);
        let config_path =
            std::env::temp_dir().join(format!("rtunes-tui-{}-{}.yaml", std::process::id(), n));
        let fetcher_settings = Arc::new(Mutex::new(cfg.fetcher.clone()));
        TuiDeps {
            config: Arc::new(Mutex::new(cfg)),
            config_path,
            fetch_pool: Arc::new(FetcherPool::new(2, Arc::new(MockFetcher))),
            fetch_tx: tx,
            picker_tx,
            fetcher_settings,
        }
    }

    fn seed_config_file(deps: &TuiDeps, cfg: &RtunesConfig) {
        crate::config::save(&deps.config_path, cfg).expect("seed config file for tests");
    }

    fn default_cfg() -> RtunesConfig {
        const YAML: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/default_config.yaml"
        ));
        serde_yaml::from_str(YAML).expect("default config")
    }

    fn sample_app() -> AppState {
        let cfg = default_cfg();
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let mut app = AppState::new(&cfg, theme);
        app.library.push(Track {
            id: "1".into(),
            filepath: PathBuf::from("a.mp3"),
            title: "Hello World".into(),
            artist: Some("Artist".into()),
            album: None,
            duration_secs: 120,
        });
        app
    }

    fn key_char(c: char) -> KeyEvent {
        KeyEvent::new(KeyCode::Char(c), KeyModifiers::NONE)
    }

    #[test]
    fn recompute_filter_empty_query_clears() {
        let mut app = sample_app();
        app.input_buffer.clear();
        recompute_filter(&mut app);
        assert!(app.filtered_indices.is_empty());
    }

    #[test]
    fn recompute_filter_matches_substring_case_insensitive() {
        let mut app = sample_app();
        app.input_buffer = "WORLD".into();
        recompute_filter(&mut app);
        assert_eq!(app.filtered_indices, vec![0]);
    }

    #[test]
    fn seek_relative_clamps() {
        let mut app = sample_app();
        app.player.position_secs = 10.0;
        app.player.duration_secs = 20.0;
        seek_relative(&mut app, 30.0);
        assert_eq!(app.player.seek_to, Some(20.0));
        app.player.position_secs = 10.0;
        seek_relative(&mut app, -100.0);
        assert_eq!(app.player.seek_to, Some(0.0));
    }

    #[test]
    fn volume_clamps_per_keystroke() {
        let mut app = sample_app();
        app.player.volume = 0.98;
        bump_volume(&mut app, 0.05);
        assert!((app.player.volume - 1.0).abs() < 1e-5);
        app.player.volume = 0.02;
        bump_volume(&mut app, -0.05);
        assert!((app.player.volume - 0.0).abs() < 1e-5);
    }

    #[test]
    fn cycle_repeat_off_to_one() {
        let mut app = sample_app();
        assert_eq!(app.player.repeat, RepeatMode::Off);
        cycle_repeat(&mut app);
        assert_eq!(app.player.repeat, RepeatMode::All);
        cycle_repeat(&mut app);
        assert_eq!(app.player.repeat, RepeatMode::One);
        cycle_repeat(&mut app);
        assert_eq!(app.player.repeat, RepeatMode::Off);
    }

    #[test]
    fn escape_closes_help_then_quits() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let app = Arc::new(Mutex::new(AppState::new(&cfg, theme)));
        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));

        let ev = Event::Key(KeyEvent::new(KeyCode::Char('?'), KeyModifiers::NONE));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        assert!(lock_shared(&app).show_help);

        let ev = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        assert!(!lock_shared(&app).show_help);
        assert!(!lock_shared(&app).quit);

        let ev = Event::Key(KeyEvent::new(KeyCode::Esc, KeyModifiers::NONE));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        assert!(lock_shared(&app).quit);
    }

    #[test]
    fn tab_cycles_panel_focus_in_normal_mode() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let app = Arc::new(Mutex::new(AppState::new(&cfg, theme)));
        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));

        assert_eq!(lock_shared(&app).panel_focus, PanelFocus::Normal);
        assert!(!lock_shared(&app).is_fullscreen);

        let tab = Event::Key(KeyEvent::new(KeyCode::Tab, KeyModifiers::NONE));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &tab).unwrap();
        assert_eq!(lock_shared(&app).panel_focus, PanelFocus::TransportOnly);
        assert!(!lock_shared(&app).is_fullscreen);

        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &tab).unwrap();
        assert!(lock_shared(&app).is_fullscreen);

        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &tab).unwrap();
        let g = lock_shared(&app);
        assert_eq!(g.panel_focus, PanelFocus::Normal);
        assert!(!g.is_fullscreen);
    }

    #[test]
    fn theme_cycle_wraps() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        let theme = resolve_active_theme("synthwave", cfg.theme.custom.as_ref());
        let app = Arc::new(Mutex::new(AppState::new(&cfg, theme)));
        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));
        for _ in 0..BUILTIN_THEME_ORDER.len() {
            let ev = Event::Key(key_char('t'));
            handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        }
        assert_eq!(
            normalize_theme_key(&lock_shared(&theme_arc).name),
            "synthwave"
        );
    }

    #[test]
    fn pressing_t_persists_theme_to_disk() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        seed_config_file(&deps, &cfg);
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let app = Arc::new(Mutex::new(AppState::new(&cfg, theme)));
        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));
        let ev = Event::Key(key_char('t'));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        let loaded = crate::config::load_or_create(&deps.config_path).unwrap();
        assert_eq!(loaded.theme.active, "dracula");
    }

    #[test]
    fn pressing_s_persists_shuffle_to_disk() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        seed_config_file(&deps, &cfg);
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let app = Arc::new(Mutex::new(AppState::new(&cfg, theme)));
        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));
        let ev = Event::Key(key_char('s'));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        let loaded = crate::config::load_or_create(&deps.config_path).unwrap();
        assert!(loaded.app.shuffle);
    }

    #[test]
    fn pressing_v_persists_visualizer_to_disk() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        seed_config_file(&deps, &cfg);
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let app = Arc::new(Mutex::new(AppState::new(&cfg, theme)));
        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));
        let ev = Event::Key(key_char('v'));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        let loaded = crate::config::load_or_create(&deps.config_path).unwrap();
        assert_eq!(loaded.app.default_visualizer, "spectrogram");
    }

    #[test]
    fn pressing_plus_persists_volume_to_disk() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        seed_config_file(&deps, &cfg);
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let mut st = AppState::new(&cfg, theme);
        st.player.volume = 0.5;
        let app = Arc::new(Mutex::new(st));
        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));
        let ev = Event::Key(KeyEvent::new(KeyCode::Char('+'), KeyModifiers::NONE));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        let loaded = crate::config::load_or_create(&deps.config_path).unwrap();
        assert!((loaded.app.volume - 0.55).abs() < 1e-5);
    }

    #[test]
    fn pressing_shift_m_persists_spectrogram_mode_to_disk() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        seed_config_file(&deps, &cfg);
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let mut st = AppState::new(&cfg, theme);
        st.visualizer_mode = VisualizerMode::Spectrogram;
        let app = Arc::new(Mutex::new(st));
        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));
        let ev = Event::Key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        let loaded = crate::config::load_or_create(&deps.config_path).unwrap();
        assert_eq!(loaded.app.spectrogram_mode, "inverted");
    }

    #[test]
    fn pressing_g_persists_neon_to_disk() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        seed_config_file(&deps, &cfg);
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let app = Arc::new(Mutex::new(AppState::new(&cfg, theme)));
        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));
        let ev = Event::Key(key_char('g'));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        let loaded = crate::config::load_or_create(&deps.config_path).unwrap();
        assert!(!loaded.app.neon);
    }

    #[test]
    fn download_prompt_rejects_invalid_url() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let app = Arc::new(Mutex::new(AppState::new(&cfg, theme.clone())));
        let theme_arc = Arc::new(Mutex::new(theme));

        let ev = Event::Key(KeyEvent::new(KeyCode::Char('d'), KeyModifiers::NONE));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        assert_eq!(lock_shared(&app).input_mode, InputMode::DownloadUrl);

        for ch in "ftp://bad.example/x".chars() {
            let ev = Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        }

        let ev = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        let g = lock_shared(&app);
        assert_eq!(g.input_mode, InputMode::Normal);
        let msg = g.message.as_ref().expect("toast").0.clone();
        assert!(
            msg.contains("http") || msg.contains("Download"),
            "unexpected message: {msg}"
        );
    }

    #[test]
    fn shift_m_cycles_spectrogram_mode_only_when_spectrogram_active() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let app = Arc::new(Mutex::new(AppState::new(&cfg, theme)));
        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));

        assert_eq!(
            lock_shared(&app).spectrogram_mode,
            SpectrogramMode::Standard
        );
        let ev = Event::Key(KeyEvent::new(KeyCode::Char('M'), KeyModifiers::SHIFT));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        assert_eq!(
            lock_shared(&app).spectrogram_mode,
            SpectrogramMode::Standard
        );

        lock_shared(&app).visualizer_mode = VisualizerMode::Spectrogram;
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        assert_eq!(
            lock_shared(&app).spectrogram_mode,
            SpectrogramMode::Inverted
        );
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        assert_eq!(
            lock_shared(&app).spectrogram_mode,
            SpectrogramMode::Mirrored
        );
    }

    #[test]
    fn library_manager_remove_clears_config_path() {
        let dir = std::env::temp_dir().join(format!("rtunes-libmgr-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let canon = dunce::canonicalize(&dir).unwrap();

        let mut cfg = default_cfg();
        cfg.app.library_paths = vec![canon.to_string_lossy().into_owned()];
        let deps = test_deps(cfg);
        let theme = resolve_active_theme("synthwave", None);
        let cfg_snapshot = lock_shared(&deps.config).clone();
        let app = Arc::new(Mutex::new(AppState::new(&cfg_snapshot, theme)));
        {
            let mut g = lock_shared(&app);
            g.library_folders = vec![LibraryFolder {
                path: canon.clone(),
                track_count: 0,
                last_scanned: None,
            }];
            g.show_library_manager = true;
            g.input_mode = InputMode::LibraryManager;
            g.selected_folder = 0;
        }

        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));
        let ev = Event::Key(KeyEvent::new(KeyCode::Char('x'), KeyModifiers::NONE));
        handle_event(&app, &theme_arc, None, &deps, &ev).unwrap();

        assert!(lock_shared(&deps.config).app.library_paths.is_empty());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn add_library_path_rejects_non_directory() {
        let cfg = default_cfg();
        let deps = test_deps(cfg.clone());
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let app = Arc::new(Mutex::new(AppState::new(&cfg, theme)));
        let theme_arc = Arc::new(Mutex::new(resolve_active_theme("synthwave", None)));

        let ev = Event::Key(KeyEvent::new(KeyCode::Char('a'), KeyModifiers::NONE));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        for ch in "/nonexistent/rtunes/folder/xyz".chars() {
            let ev = Event::Key(KeyEvent::new(KeyCode::Char(ch), KeyModifiers::NONE));
            handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        }
        let ev = Event::Key(KeyEvent::new(KeyCode::Enter, KeyModifiers::NONE));
        handle_event(&app, &theme_arc, cfg.theme.custom.as_ref(), &deps, &ev).unwrap();
        let g = lock_shared(&app);
        assert!(g.message.as_ref().unwrap().0.contains("Not a directory"));
    }
}
