//! yt-dlp subprocess wrapper, URL validation, and a [`MockFetcher`] for tests.

use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use url::Url;

use crate::config::FetcherSettings;
use crate::error::{Result, RtunesError};
use crate::utils::{expand_path, resolve_binary};

/// Progress / completion events from a download worker (sent over a channel).
#[derive(Debug, Clone)]
pub enum FetchEvent {
    Stage(String),
    Progress(f32),
    Done(PathBuf),
    Failed(String),
    /// yt-dlp / ffmpeg not found; user must confirm before auto-download begins.
    DepsPrompt(Vec<MissingTool>),
    /// A dep binary is being downloaded; progress in [0.0, 1.0].
    DepsDownloading { tool: String, progress: f32 },
    /// All required deps are now present; the pending fetch can be retried.
    DepsReady,
}

/// Per-download options (format string passed to yt-dlp, output directory for artifacts).
#[derive(Debug, Clone)]
pub struct FetchOpts {
    pub format: String,
    pub output_dir: PathBuf,
}

/// Pluggable download backend (real yt-dlp or [`MockFetcher`] for tests).
///
/// Implementations should send at least one terminal [`FetchEvent::Done`] or
/// [`FetchEvent::Failed`] before returning.
pub trait Fetcher: Send + Sync {
    /// Run the download; push progress to `tx` (non-blocking `send` is fine).
    fn fetch(&self, url: &Url, opts: &FetchOpts, tx: Sender<FetchEvent>) -> Result<()>;
}

/// Reject non-http(s) URLs and suspicious shell metacharacters (`;`, `|`).
///
/// Accepts typical streaming links with query strings (e.g. YouTube `watch?v=…&list=…`).
pub fn validate_url(raw: &str) -> Result<Url> {
    let s = raw.trim();
    if s.contains(';') || s.contains('|') {
        return Err(RtunesError::Fetcher(
            "URL contains forbidden characters (; or |)".into(),
        ));
    }
    let u = Url::parse(s).map_err(|e| RtunesError::Fetcher(e.to_string()))?;
    match u.scheme() {
        "http" | "https" => Ok(u),
        _ => Err(RtunesError::Fetcher("only http(s) URLs are allowed".into())),
    }
}

fn resolve_tool_path(cfg_entry: &str, binary_name: &str) -> Option<PathBuf> {
    if cfg_entry == "auto" {
        resolve_binary(binary_name)
    } else {
        let p = expand_path(cfg_entry);
        if p.is_file() {
            return Some(p);
        }
        // If the configured value is a directory, search inside it for the binary.
        if p.is_dir() {
            return crate::utils::find_binary_in_dir(&p, binary_name);
        }
        None
    }
}

/// Public wrapper around `resolve_tool_path` for use by the TUI snapshot builder.
///
/// Returns the resolved path for a fetcher tool config entry, or `None` if not found.
pub fn resolve_fetcher_tool(cfg_entry: &str, binary_name: &str) -> Option<PathBuf> {
    resolve_tool_path(cfg_entry, binary_name)
}

/// Which binary is missing when [`try_resolve_tools`] fails.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MissingTool {
    YtDlp,
    Ffmpeg,
}

/// Resolve both yt-dlp and ffmpeg from `settings`.
///
/// Returns `Ok((ytdlp_path, ffmpeg_path))` when both are found, or
/// `Err(missing)` listing whichever tools could not be located.
pub fn try_resolve_tools(
    settings: &FetcherSettings,
) -> std::result::Result<(PathBuf, PathBuf), Vec<MissingTool>> {
    let ytdlp = resolve_tool_path(&settings.ytdlp_path, "yt-dlp");
    let ffmpeg = resolve_tool_path(&settings.ffmpeg_path, "ffmpeg");
    match (ytdlp, ffmpeg) {
        (Some(y), Some(f)) => Ok((y, f)),
        (yt, ff) => {
            let mut missing = Vec::new();
            if yt.is_none() {
                missing.push(MissingTool::YtDlp);
            }
            if ff.is_none() {
                missing.push(MissingTool::Ffmpeg);
            }
            Err(missing)
        }
    }
}

/// ffmpeg location for yt-dlp: parent directory of the ffmpeg binary.
fn ffmpeg_location_dir(ffmpeg: &Path) -> PathBuf {
    ffmpeg
        .parent()
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("."))
}

fn parse_download_percent(line: &str) -> Option<f32> {
    if !line.contains("[download]") {
        return None;
    }
    let idx = line.find('%')?;
    let head = line[..idx].trim_end();
    let num = head.split_whitespace().last()?.parse::<f32>().ok()?;
    Some((num / 100.0).clamp(0.0, 1.0))
}

fn parse_destination(line: &str) -> Option<PathBuf> {
    const PREFIXES: [&str; 2] = ["[ExtractAudio] Destination: ", "[download] Destination: "];
    for p in PREFIXES {
        if let Some(rest) = line.strip_prefix(p) {
            let path = rest.trim().trim_matches('"');
            if !path.is_empty() {
                return Some(PathBuf::from(path));
            }
        }
    }
    None
}

pub struct YtDlpFetcher {
    pub settings: Arc<Mutex<FetcherSettings>>,
}

impl YtDlpFetcher {
    pub fn new(settings: Arc<Mutex<FetcherSettings>>) -> Self {
        Self { settings }
    }
}

impl Fetcher for YtDlpFetcher {
    fn fetch(&self, url: &Url, opts: &FetchOpts, tx: Sender<FetchEvent>) -> Result<()> {
        // Read a fresh snapshot each call so runtime path changes (Settings overlay,
        // CLI picker) take effect immediately without restarting.
        let settings = self.settings.lock().unwrap().clone();
        let ytdlp = match resolve_tool_path(&settings.ytdlp_path, "yt-dlp") {
            Some(p) => p,
            None => {
                let _ = tx.send(FetchEvent::Failed(
                    "yt-dlp not found. Place it in deps/ next to rtunes.exe, install it to PATH, \
                     or press F2 in the TUI to configure the path."
                        .into(),
                ));
                return Ok(());
            }
        };
        let ffmpeg = match resolve_tool_path(&settings.ffmpeg_path, "ffmpeg") {
            Some(p) => p,
            None => {
                let _ = tx.send(FetchEvent::Failed(
                    "ffmpeg not found. Place it in deps/ next to rtunes.exe, install it to PATH, \
                     or press F2 in the TUI to configure the path."
                        .into(),
                ));
                return Ok(());
            }
        };

        fs::create_dir_all(&opts.output_dir).map_err(|e| {
            RtunesError::Fetcher(format!(
                "create output dir {}: {e}",
                opts.output_dir.display()
            ))
        })?;

        let out_tmpl = opts
            .output_dir
            .join("%(title)s.%(ext)s")
            .to_string_lossy()
            .to_string();

        let ffmpeg_dir = ffmpeg_location_dir(&ffmpeg);

        let mut child = Command::new(&ytdlp)
            .arg("-x")
            .arg("--audio-format")
            .arg(opts.format.trim())
            .arg("--newline")
            .arg("--ffmpeg-location")
            .arg(&ffmpeg_dir)
            .arg("-o")
            .arg(&out_tmpl)
            .arg(url.as_str())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| RtunesError::Fetcher(format!("spawn yt-dlp: {e}")))?;

        // yt-dlp writes --newline progress to stderr; drain stdout so the pipe never blocks.
        let stdout = child.stdout.take();
        let _drain_out = std::thread::spawn(move || {
            if let Some(r) = stdout {
                for _ in BufReader::new(r).lines().map_while(|l| l.ok()) {}
            }
        });

        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| RtunesError::Fetcher("yt-dlp stderr not captured".into()))?;

        let mut last_path: Option<PathBuf> = None;
        let mut stderr_full = String::new();
        for line in BufReader::new(stderr).lines().map_while(|l| l.ok()) {
            stderr_full.push_str(&line);
            stderr_full.push('\n');
            if let Some(pct) = parse_download_percent(&line) {
                let _ = tx.send(FetchEvent::Progress(pct));
            }
            if line.starts_with('[') {
                let _ = tx.send(FetchEvent::Stage(line.clone()));
            }
            if let Some(p) = parse_destination(&line) {
                last_path = Some(p);
            }
        }

        let status = child
            .wait()
            .map_err(|e| RtunesError::Fetcher(e.to_string()))?;
        let _ = _drain_out.join();
        let stderr_text = stderr_full;
        if !status.success() {
            let tail = stderr_text.chars().rev().take(800).collect::<String>();
            let tail = tail.chars().rev().collect::<String>();
            let msg = if tail.trim().is_empty() {
                format!("yt-dlp exited with {status}")
            } else {
                format!("yt-dlp failed ({status}): {}", tail.trim())
            };
            let _ = tx.send(FetchEvent::Failed(msg));
            return Ok(());
        }

        let path = match last_path {
            Some(p) if p.exists() => p,
            Some(p) => {
                let _ = tx.send(FetchEvent::Failed(format!(
                    "expected output file missing: {}",
                    p.display()
                )));
                return Ok(());
            }
            None => {
                let _ = tx.send(FetchEvent::Failed(
                    "download finished but output path was not detected".into(),
                ));
                return Ok(());
            }
        };

        let _ = tx.send(FetchEvent::Done(path));
        Ok(())
    }
}

/// Test fetcher: writes a tiny stub `.mp3` and emits scripted events.
#[allow(dead_code)] // Used from `#[cfg(test)]` modules and `tests/integration`.
pub struct MockFetcher;

impl Fetcher for MockFetcher {
    fn fetch(&self, url: &Url, opts: &FetchOpts, tx: Sender<FetchEvent>) -> Result<()> {
        fs::create_dir_all(&opts.output_dir).map_err(|e| {
            RtunesError::Fetcher(format!(
                "create output dir {}: {e}",
                opts.output_dir.display()
            ))
        })?;

        let stem = url
            .path_segments()
            .and_then(|mut s| s.next_back().map(|s| s.to_string()))
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| "mock_track".into());
        let safe: String = stem
            .chars()
            .filter(|c| c.is_alphanumeric() || *c == '-' || *c == '_')
            .take(64)
            .collect();
        let name = if safe.is_empty() {
            "mock_track".into()
        } else {
            safe
        };
        let path = opts.output_dir.join(format!("{name}.mp3"));
        fs::write(&path, b"ID3\x03\x00\x00\x00\x00\x00\x00mock")?;

        let _ = tx.send(FetchEvent::Stage("Downloading".into()));
        let _ = tx.send(FetchEvent::Progress(0.5));
        let _ = tx.send(FetchEvent::Progress(1.0));
        let _ = tx.send(FetchEvent::Done(path));
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_url_accepts_https_with_query() {
        let u = validate_url("https://www.youtube.com/watch?v=abc&list=xyz").unwrap();
        assert_eq!(u.scheme(), "https");
    }

    #[test]
    fn validate_url_accepts_youtube_with_amp_query() {
        let u = validate_url("https://www.youtube.com/watch?v=abc&list=def").unwrap();
        assert_eq!(u.query_pairs().count(), 2);
        assert_eq!(u.scheme(), "https");
    }

    #[test]
    fn validate_url_rejects_ftp() {
        assert!(validate_url("ftp://example.com/x").is_err());
    }

    #[test]
    fn validate_url_rejects_file_scheme() {
        assert!(validate_url("file:///etc/passwd").is_err());
    }

    #[test]
    fn validate_url_rejects_semicolon() {
        assert!(validate_url("https://a.com/x;rm -rf").is_err());
    }

    #[test]
    fn validate_url_rejects_pipe() {
        assert!(validate_url("https://a.com/x|evil").is_err());
    }

    #[test]
    fn mock_fetcher_emits_done() {
        let dir = std::env::temp_dir().join(format!("rtunes-mock-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let (tx, rx) = crossbeam_channel::unbounded();
        let f = MockFetcher;
        let url = Url::parse("https://example.com/audio/stub").unwrap();
        let opts = FetchOpts {
            format: "mp3".into(),
            output_dir: dir.clone(),
        };
        f.fetch(&url, &opts, tx).unwrap();
        let mut saw_done = false;
        while let Ok(ev) = rx.recv_timeout(std::time::Duration::from_secs(2)) {
            if matches!(ev, FetchEvent::Done(_)) {
                saw_done = true;
                break;
            }
        }
        assert!(saw_done);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn resolve_tool_path_accepts_directory() {
        let dir = std::env::temp_dir().join(format!("rtunes-dir-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        // Create a fake binary file with the platform-appropriate name.
        #[cfg(windows)]
        let bin_name = "yt-dlp.exe";
        #[cfg(not(windows))]
        let bin_name = "yt-dlp";
        let bin_path = dir.join(bin_name);
        std::fs::write(&bin_path, b"fake").unwrap();

        // Passing the directory as the config entry should resolve to the binary inside it.
        let resolved = resolve_tool_path(&dir.to_string_lossy(), "yt-dlp");
        assert!(resolved.is_some(), "should resolve binary inside directory");
        assert_eq!(resolved.unwrap(), bin_path);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn try_resolve_tools_reports_missing_when_both_absent() {
        use crate::config::FetcherSettings;
        let settings = FetcherSettings {
            ytdlp_path: "/nonexistent/yt-dlp-xyzzy".into(),
            ffmpeg_path: "/nonexistent/ffmpeg-xyzzy".into(),
            default_format: "mp3".into(),
            max_concurrent: 1,
        };
        let result = try_resolve_tools(&settings);
        assert!(result.is_err());
        let missing = result.unwrap_err();
        assert!(missing.contains(&MissingTool::YtDlp));
        assert!(missing.contains(&MissingTool::Ffmpeg));
    }

    #[test]
    fn try_resolve_tools_ok_when_both_present() {
        let dir = std::env::temp_dir().join(format!("rtunes-try-resolve-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();

        #[cfg(windows)]
        let (ytdlp_name, ffmpeg_name) = ("yt-dlp.exe", "ffmpeg.exe");
        #[cfg(not(windows))]
        let (ytdlp_name, ffmpeg_name) = ("yt-dlp", "ffmpeg");

        std::fs::write(dir.join(ytdlp_name), b"fake").unwrap();
        std::fs::write(dir.join(ffmpeg_name), b"fake").unwrap();

        use crate::config::FetcherSettings;
        let settings = FetcherSettings {
            ytdlp_path: dir.join(ytdlp_name).to_string_lossy().into_owned(),
            ffmpeg_path: dir.join(ffmpeg_name).to_string_lossy().into_owned(),
            default_format: "mp3".into(),
            max_concurrent: 1,
        };
        let result = try_resolve_tools(&settings);
        assert!(result.is_ok(), "both binaries present should succeed");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
