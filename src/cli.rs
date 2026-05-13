//! CLI (clap). Subcommand dispatch for scan / library; stubs for tui / fetch.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use clap::{Parser, Subcommand};
use crossbeam_channel;

use crate::app;
use crate::audio::{AudioPlayer, RodioBackend};
use crate::config::{self, RtunesConfig, Theme};
use crate::fetcher::{
    FetchEvent, FetchOpts, Fetcher, FetcherPool, MissingTool, PickerEvent,
    YtDlpFetcher,
};
use crate::library::{scan_config_paths, scan_paths};
use crate::tui;
use crate::tui::events::TuiDeps;
use crate::visualizer;

#[derive(Parser)]
#[command(
    name = "rtunes",
    version,
    about = "Terminal music player with visualizers"
)]
pub struct Cli {
    /// Path to config.yaml (overrides default beside the executable / `RTUNES_CONFIG_PATH`).
    #[arg(long, global = true, value_name = "PATH")]
    pub config: Option<PathBuf>,

    /// Log level: error, warn, info, debug, trace (overrides RTUNES_LOG_LEVEL).
    #[arg(long, global = true, value_name = "LEVEL")]
    pub log_level: Option<String>,

    /// Theme name (default TUI launch; merged with `tui` subcommand if given).
    #[arg(long, global = true)]
    pub theme: Option<String>,

    /// Target render FPS, 30–60 (default TUI launch).
    #[arg(long, global = true, value_name = "NUM")]
    pub fps: Option<u8>,

    /// Start in fullscreen visualizer mode (default TUI launch).
    #[arg(long, global = true)]
    pub fullscreen: bool,

    /// Explicit path to the yt-dlp binary (overrides `fetcher.ytdlp_path` in config).
    #[arg(long, global = true, value_name = "PATH")]
    pub ytdlp_path: Option<PathBuf>,

    /// Explicit path to the ffmpeg binary (overrides `fetcher.ffmpeg_path` in config).
    #[arg(long, global = true, value_name = "PATH")]
    pub ffmpeg_path: Option<PathBuf>,

    #[command(subcommand)]
    pub command: Option<Commands>,
}

#[derive(Subcommand)]
pub enum Commands {
    /// Launch the terminal UI.
    Tui {
        /// Theme name (overrides config for this session).
        #[arg(long)]
        theme: Option<String>,
        /// Target render FPS, 30–60.
        #[arg(long, value_name = "NUM")]
        fps: Option<u8>,
        /// Start in fullscreen visualizer mode.
        #[arg(long)]
        fullscreen: bool,
    },
    /// Download audio from a URL via yt-dlp.
    Fetch {
        /// Media URL for yt-dlp.
        url: String,
        /// Audio format (default from config).
        #[arg(long)]
        format: Option<String>,
        /// Output directory (default: first library path, else download_dir).
        #[arg(long, value_name = "DIR")]
        output: Option<PathBuf>,
    },
    /// Rebuild library index from all configured folders.
    Scan,
    /// Manage library folders.
    Library {
        #[command(subcommand)]
        cmd: LibraryCmd,
    },
}

#[derive(Subcommand)]
pub enum LibraryCmd {
    /// Add a folder to the library paths.
    Add {
        /// Directory to add.
        path: PathBuf,
    },
    /// Remove a folder from the library paths.
    Remove {
        /// Directory to remove.
        path: PathBuf,
    },
    /// List configured library folders.
    List,
}

/// Canonical, deduplicated existing directory roots from config path strings.
pub fn resolve_library_roots(paths: &[String]) -> Vec<PathBuf> {
    use std::collections::HashSet;

    let mut out = Vec::new();
    let mut seen: HashSet<PathBuf> = HashSet::new();
    for s in paths {
        let exp = crate::utils::expand_path(s);
        if let Ok(c) = dunce::canonicalize(&exp) {
            if c.is_dir() && seen.insert(c.clone()) {
                out.push(c);
            }
        }
    }
    out
}

fn validate_dir(path: &Path) -> anyhow::Result<PathBuf> {
    let expanded = crate::utils::expand_path(&path.to_string_lossy());
    let md =
        std::fs::metadata(&expanded).map_err(|e| anyhow::anyhow!("{}: {e}", expanded.display()))?;
    if !md.is_dir() {
        anyhow::bail!("not a directory: {}", expanded.display());
    }
    dunce::canonicalize(&expanded).map_err(|e| anyhow::anyhow!("{}: {e}", expanded.display()))
}

fn canonical_of_config_entry(s: &str) -> Option<PathBuf> {
    let exp = crate::utils::expand_path(s);
    dunce::canonicalize(&exp).ok()
}

fn sync_library_folder_counts(state: &mut crate::app::state::AppState) {
    let now = Instant::now();
    for folder in &mut state.library_folders {
        let p = folder.path.as_path();
        folder.track_count = state
            .library
            .iter()
            .filter(|t| t.filepath.starts_with(p))
            .count();
        folder.last_scanned = Some(now);
    }
}

fn run_tui_session(
    cfg_path: &Path,
    cfg: &mut RtunesConfig,
    theme_cli: &Option<String>,
    fps_cli: &Option<u8>,
    fullscreen: bool,
) -> anyhow::Result<()> {
    if let Some(t) = theme_cli {
        cfg.theme.active = t.clone();
    }
    if let Some(f) = fps_cli {
        cfg.app.fps = (*f).clamp(30, 60);
    }
    if fullscreen {
        cfg.app.start_fullscreen = true;
    }

    let resolved = config::resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
    let state = app::new_shared_state(cfg, resolved.clone());

    {
        let mut g = state.lock().unwrap();
        g.library = scan_config_paths(&cfg.app.library_paths);
        sync_library_folder_counts(&mut g);
    }

    let theme_arc = Arc::new(Mutex::new(resolved));
    let custom: Option<Arc<HashMap<String, Theme>>> = cfg.theme.custom.clone().map(Arc::new);

    let shared_cfg = Arc::new(Mutex::new(cfg.clone()));
    // Share FetcherSettings so the live YtDlpFetcher picks up path changes immediately.
    let shared_fetcher_settings = Arc::new(Mutex::new(shared_cfg.lock().unwrap().fetcher.clone()));
    let fetcher_impl: Arc<dyn Fetcher + Send + Sync> =
        Arc::new(YtDlpFetcher::new(shared_fetcher_settings.clone()));
    let max_c = shared_cfg.lock().unwrap().fetcher.max_concurrent.max(1) as usize;
    let fetch_pool = Arc::new(FetcherPool::new(max_c, fetcher_impl));
    let (fetch_tx, fetch_rx) = crossbeam_channel::unbounded();
    let (picker_tx, picker_rx) = crossbeam_channel::unbounded::<PickerEvent>();
    let deps = TuiDeps {
        config: shared_cfg.clone(),
        config_path: cfg_path.to_path_buf(),
        fetch_pool,
        fetch_tx,
        picker_tx,
        fetcher_settings: shared_fetcher_settings,
    };

    let ring_size = cfg.audio.ring_buffer_size as usize;
    let (audio_join, ring, samples, sample_rate_hz, silent) =
        AudioPlayer::<RodioBackend>::spawn(state.clone(), ring_size);

    let fft = visualizer::spawn_fft_thread(
        state.clone(),
        ring,
        samples,
        sample_rate_hz,
        cfg.audio.clone(),
    );

    let run_result = tui::run_tui(
        state.clone(),
        theme_arc,
        shared_cfg.clone(),
        cfg_path.to_path_buf(),
        custom,
        deps,
        fetch_rx,
        picker_rx,
        cfg.app.fps,
        silent,
        fft.rx,
    );

    {
        let mut g = state.lock().unwrap();
        g.quit = true;
    }

    // Bounded shutdown: poll with 50ms intervals up to 2 s, then fall through.
    // This prevents a stuck audio/FFT thread from blocking the quit path indefinitely.
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
    let mut audio_handle = Some(audio_join);
    let mut fft_handle = Some(fft.join);
    loop {
        let audio_done = audio_handle
            .as_ref()
            .map(|h| h.is_finished())
            .unwrap_or(true);
        let fft_done = fft_handle.as_ref().map(|h| h.is_finished()).unwrap_or(true);
        if audio_done && fft_done {
            break;
        }
        if std::time::Instant::now() >= deadline {
            tracing::warn!("shutdown timeout: audio_done={audio_done} fft_done={fft_done}");
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    if let Some(h) = audio_handle.take() {
        if h.is_finished() {
            let _ = h.join();
        }
    }
    if let Some(h) = fft_handle.take() {
        if h.is_finished() {
            let _ = h.join();
        }
    }

    *cfg = shared_cfg.lock().unwrap().clone();

    run_result
}

/// Dispatch subcommands. Mutates `cfg` and saves for `scan` / `library`.
pub fn dispatch(cli: &Cli, cfg_path: &Path, cfg: &mut RtunesConfig) -> anyhow::Result<()> {
    // Apply any CLI-level binary path overrides before dispatch.
    if let Some(ref p) = cli.ytdlp_path {
        cfg.fetcher.ytdlp_path = p.to_string_lossy().into_owned();
    }
    if let Some(ref p) = cli.ffmpeg_path {
        cfg.fetcher.ffmpeg_path = p.to_string_lossy().into_owned();
    }

    match &cli.command {
        None => run_tui_session(cfg_path, cfg, &cli.theme, &cli.fps, cli.fullscreen)?,
        Some(Commands::Tui {
            theme,
            fps,
            fullscreen,
        }) => {
            let theme_merged = theme.clone().or_else(|| cli.theme.clone());
            let fps_merged = fps.or(cli.fps);
            let fullscreen_merged = *fullscreen || cli.fullscreen;
            run_tui_session(cfg_path, cfg, &theme_merged, &fps_merged, fullscreen_merged)?;
        }
        Some(Commands::Fetch {
            url,
            format: format_arg,
            output,
        }) => {
            use crate::fetcher::{
                deps_dir, download_ffmpeg, download_ytdlp,
                ffmpeg_auto_download_supported, ffmpeg_manual_instructions,
                try_resolve_tools, validate_url,
            };
            use std::io::{self, BufRead, Write};

            // Pre-flight: check whether yt-dlp and ffmpeg are available.
            if let Err(missing) = try_resolve_tools(&cfg.fetcher) {
                let names: Vec<&str> = missing
                    .iter()
                    .map(|t| match t {
                        MissingTool::YtDlp => "yt-dlp",
                        MissingTool::Ffmpeg => "ffmpeg",
                    })
                    .collect();
                let list = names.join(" and ");
                eprint!("{list} not found. Download automatically? (~120MB) [Y/n]: ");
                let _ = io::stderr().flush();
                let stdin = io::stdin();
                let answer = stdin.lock().lines().next().and_then(|l| l.ok()).unwrap_or_default();
                if !matches!(answer.trim().to_lowercase().as_str(), "y" | "yes" | "") {
                    anyhow::bail!("{list} is required. Install manually or use --ytdlp-path / --ffmpeg-path.");
                }

                let dd = deps_dir().ok_or_else(|| anyhow::anyhow!("Could not determine deps/ directory"))?;

                for tool in &missing {
                    match tool {
                        MissingTool::YtDlp => {
                            eprintln!("Downloading yt-dlp…");
                            download_ytdlp(&dd, |p| {
                                eprint!("\r  {:>5.1}%", (p * 100.0).min(100.0));
                                let _ = io::stderr().flush();
                            })
                            .map_err(|e| anyhow::anyhow!(e))?;
                            eprintln!("\r  yt-dlp ready.           ");
                        }
                        MissingTool::Ffmpeg => {
                            if !ffmpeg_auto_download_supported() {
                                anyhow::bail!("ffmpeg not found. {}", ffmpeg_manual_instructions());
                            }
                            eprintln!("Downloading ffmpeg…");
                            download_ffmpeg(&dd, |p| {
                                eprint!("\r  {:>5.1}%", (p * 100.0).min(100.0));
                                let _ = io::stderr().flush();
                            })
                            .map_err(|e| anyhow::anyhow!(e))?;
                            eprintln!("\r  ffmpeg ready.           ");
                        }
                    }
                }
                // Reset to "auto" so resolve_binary finds the freshly downloaded binaries.
                cfg.fetcher.ytdlp_path = "auto".into();
                cfg.fetcher.ffmpeg_path = "auto".into();
            }

            let fmt = format_arg
                .clone()
                .unwrap_or_else(|| cfg.fetcher.default_format.clone());
            let out_dir = if let Some(ref o) = output {
                crate::utils::expand_path(&o.to_string_lossy())
            } else {
                resolve_library_roots(&cfg.app.library_paths)
                    .into_iter()
                    .next()
                    .unwrap_or_else(|| crate::utils::expand_path(&cfg.app.download_dir))
            };
            let u = validate_url(url).map_err(|e| anyhow::anyhow!(e.to_string()))?;
            let (tx, rx) = crossbeam_channel::unbounded();
            let fetcher = YtDlpFetcher::new(Arc::new(Mutex::new(cfg.fetcher.clone())));
            fetcher
                .fetch(
                    &u,
                    &FetchOpts {
                        format: fmt,
                        output_dir: out_dir,
                    },
                    tx,
                )
                .map_err(|e| anyhow::anyhow!(e.to_string()))?;
            while let Ok(ev) = rx.recv() {
                match ev {
                    FetchEvent::Progress(p) => {
                        eprint!("\rDownload: {:>5.1}%", (p * 100.0).min(100.0));
                    }
                    FetchEvent::Stage(s) => {
                        eprintln!("\n{s}");
                    }
                    FetchEvent::Done(p) => {
                        eprintln!("\nSaved: {}", p.display());
                        break;
                    }
                    FetchEvent::Failed(m) => {
                        eprintln!("\n{m}");
                        std::process::exit(1);
                    }
                    // Deps events don't occur in CLI mode (deps are resolved above).
                    FetchEvent::DepsPrompt(_)
                    | FetchEvent::DepsDownloading { .. }
                    | FetchEvent::DepsReady => {}
                }
            }
        }
        Some(Commands::Scan) => {
            let roots = resolve_library_roots(&cfg.app.library_paths);
            let n = roots.len();
            let tracks = scan_paths(&roots);
            println!("Scanned {n} folders. Found {} tracks.", tracks.len());
        }
        Some(Commands::Library { cmd }) => match cmd {
            LibraryCmd::Add { path } => {
                let canon = match validate_dir(path) {
                    Ok(c) => c,
                    Err(e) => {
                        eprintln!("{e}");
                        return Ok(());
                    }
                };
                for existing in &cfg.app.library_paths {
                    if let Some(ec) = canonical_of_config_entry(existing) {
                        if ec == canon {
                            println!("Already in library: {existing}");
                            return Ok(());
                        }
                    }
                }
                let store = canon.to_string_lossy().into_owned();
                cfg.app.library_paths.push(store);
                config::save(cfg_path, cfg).map_err(|e| anyhow::anyhow!(e.to_string()))?;
                println!("Added: {}", canon.display());
                let tracks = scan_config_paths(&cfg.app.library_paths);
                println!("Reindexing... Found {} tracks total.", tracks.len());
            }
            LibraryCmd::Remove { path } => {
                let exp = crate::utils::expand_path(&path.to_string_lossy());
                let by_canon = dunce::canonicalize(&exp).ok();
                let mut removed: Option<String> = None;
                if let Some(ref target) = by_canon {
                    for (i, s) in cfg.app.library_paths.iter().enumerate() {
                        if canonical_of_config_entry(s).as_ref() == Some(target) {
                            removed = Some(cfg.app.library_paths.remove(i));
                            break;
                        }
                    }
                }
                if removed.is_none() {
                    for (i, s) in cfg.app.library_paths.iter().enumerate() {
                        if crate::utils::expand_path(s) == exp {
                            removed = Some(cfg.app.library_paths.remove(i));
                            break;
                        }
                    }
                }
                match removed {
                    Some(s) => {
                        config::save(cfg_path, cfg).map_err(|e| anyhow::anyhow!(e.to_string()))?;
                        println!("Removed: {s}");
                        let tracks = scan_config_paths(&cfg.app.library_paths);
                        println!("Reindexing... Found {} tracks total.", tracks.len());
                    }
                    None => println!("Not in library: {}", path.display()),
                }
            }
            LibraryCmd::List => {
                println!("Library folders:");
                for (i, s) in cfg.app.library_paths.iter().enumerate() {
                    let exp = crate::utils::expand_path(s);
                    let count = if let Ok(root) = dunce::canonicalize(&exp) {
                        if root.is_dir() {
                            scan_paths(&[root]).len()
                        } else {
                            0
                        }
                    } else {
                        0
                    };
                    println!("  {}. {} ({count} tracks)", i + 1, s);
                }
            }
        },
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::Cli;
    use clap::Parser;

    #[test]
    fn cli_with_no_args_parses_to_none_command() {
        let c = Cli::try_parse_from(["rtunes"]).unwrap();
        assert!(c.command.is_none());
    }

    #[test]
    fn cli_with_top_level_theme_flag_parses() {
        let c = Cli::try_parse_from(["rtunes", "--theme", "synthwave"]).unwrap();
        assert_eq!(c.theme.as_deref(), Some("synthwave"));
        assert!(c.command.is_none());
    }
}
