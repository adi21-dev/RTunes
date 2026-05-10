//! Recursive library scan, metadata via lofty, canonical deduplication.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crossbeam_channel::Sender;
use lofty::file::{AudioFile, TaggedFileExt};
use lofty::read_from_path;
use lofty::tag::Accessor;

use crate::app::state::{track_id_for_path, Track};
use crate::utils;

/// Recognized audio file extensions (lowercase, no leading dot).
pub const AUDIO_EXTENSIONS: &[&str] =
    &["mp3", "flac", "wav", "m4a", "opus", "ogg", "aac"];

/// Case-insensitive extension check (expects `ext` without leading `.`).
pub fn is_audio_extension(ext: &str) -> bool {
    let e = ext.to_ascii_lowercase();
    AUDIO_EXTENSIONS.iter().any(|&a| a == e.as_str())
}

/// Remove `[...]` segments (YouTube-style tags, ids) without regex.
fn strip_square_bracket_segments(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0i32;
    for c in s.chars() {
        match c {
            '[' => depth += 1,
            ']' => depth = (depth - 1).max(0),
            _ if depth == 0 => out.push(c),
            _ => {}
        }
    }
    out
}

/// Collapse `Artist - Topic - Title` style filenames before splitting.
fn normalize_topic_dashes(s: &str) -> String {
    let mut t = s.to_string();
    for pat in [" - Topic - ", " - topic - ", " - TOPIC - "] {
        if t.contains(pat) {
            t = t.replace(pat, " - ");
            break;
        }
    }
    t
}

/// Parse `Artist - Title` from a filename stem; used when tags are missing.
pub(crate) fn clean_filename_title(stem: &str) -> (Option<String>, String) {
    let s = strip_square_bracket_segments(stem);
    let s = normalize_topic_dashes(&s);
    let s = s.trim();
    if let Some(pos) = s.find(" - ") {
        let left = s[..pos].trim();
        let right = s[pos + 3..].trim();
        if !left.is_empty() && !right.is_empty() {
            return (Some(left.to_string()), right.to_string());
        }
    }
    (None, s.to_string())
}

fn walk_dir(root: &Path, out: &mut Vec<PathBuf>) {
    let read_dir = match std::fs::read_dir(root) {
        Ok(rd) => rd,
        Err(e) => {
            tracing::warn!(path = %root.display(), error = %e, "read_dir failed — check folder exists and you have read permission");
            return;
        }
    };

    for entry in read_dir.flatten() {
        let path = entry.path();
        let ft = match entry.file_type() {
            Ok(t) => t,
            Err(e) => {
                tracing::warn!(path = %path.display(), error = %e, "file_type failed — skipping entry");
                continue;
            }
        };
        if ft.is_symlink() {
            continue;
        }
        if ft.is_dir() {
            walk_dir(&path, out);
        } else if ft.is_file() {
            if let Some(ext) = path.extension().and_then(|e| e.to_str()) {
                if is_audio_extension(ext) {
                    out.push(path);
                }
            }
        }
    }
}

fn extract_metadata(path: &Path) -> Track {
    let canonical = dunce::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let fallback_title = path
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_default();

    match read_from_path(&canonical) {
        Ok(tagged) => {
            let duration_secs = tagged.properties().duration().as_secs();
            let (title, artist, album) = tagged
                .primary_tag()
                .or_else(|| tagged.first_tag())
                .map(|tag| {
                    let tag_artist = tag.artist().map(|a| a.to_string());
                    let title_opt = tag
                        .title()
                        .map(|t| t.to_string())
                        .filter(|s| !s.is_empty());
                    match title_opt {
                        Some(t) => (t, tag_artist, tag.album().map(|a| a.to_string())),
                        None => {
                            let (a, t) = clean_filename_title(&fallback_title);
                            (t, tag_artist.or(a), tag.album().map(|a| a.to_string()))
                        }
                    }
                })
                .unwrap_or_else(|| {
                    let (a, t) = clean_filename_title(&fallback_title);
                    (t, a, None)
                });

            Track {
                id: track_id_for_path(&canonical),
                filepath: canonical,
                title,
                artist,
                album,
                duration_secs,
            }
        }
        Err(e) => {
            tracing::warn!(path = %canonical.display(), error = %e, "metadata read failed; indexing with filename only (file may be corrupt or unsupported)");
            let (artist, title) = clean_filename_title(&fallback_title);
            Track {
                id: track_id_for_path(&canonical),
                filepath: canonical,
                title,
                artist,
                album: None,
                duration_secs: 0,
            }
        }
    }
}

/// Expand config-style path strings then scan (see [`scan_paths`]).
pub fn scan_config_paths(paths: &[String]) -> Vec<Track> {
    let expanded: Vec<PathBuf> = paths.iter().map(|s| utils::expand_path(s)).collect();
    scan_paths(&expanded)
}

/// Walk configured roots, dedupe by canonical file path, return sorted tracks.
///
/// Skips unreadable paths with warnings; corrupt tags fall back to filename (see logs).
pub fn scan_paths(paths: &[PathBuf]) -> Vec<Track> {
    let mut unique_roots: Vec<PathBuf> = Vec::new();
    let mut seen_roots: HashSet<PathBuf> = HashSet::new();

    for p in paths {
        let expanded = utils::expand_path(&p.to_string_lossy());
        match dunce::canonicalize(&expanded) {
            Ok(canon) => {
                if canon.is_dir() {
                    if seen_roots.insert(canon.clone()) {
                        unique_roots.push(canon);
                    }
                } else {
                    tracing::warn!(path = %expanded.display(), "library path is not a directory — use `rtunes library add <dir>` or fix config.yaml");
                }
            }
            Err(e) => {
                tracing::warn!(path = %expanded.display(), error = %e, "canonicalize failed — path missing or permission denied; fix library_paths in config");
            }
        }
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for root in &unique_roots {
        walk_dir(root, &mut files);
    }

    let mut seen_files: HashSet<PathBuf> = HashSet::new();
    let mut unique_files: Vec<PathBuf> = Vec::new();
    for f in files {
        match dunce::canonicalize(&f) {
            Ok(c) => {
                if seen_files.insert(c.clone()) {
                    unique_files.push(c);
                }
            }
            Err(e) => {
                tracing::warn!(path = %f.display(), error = %e, "skip file (cannot canonicalize — broken symlink?)");
            }
        }
    }

    unique_files.sort();
    unique_files.into_iter().map(|p| extract_metadata(&p)).collect()
}

/// Progress and completion events from a background scan (`scan_async`).
#[derive(Debug, Clone)]
#[allow(dead_code)]
pub enum ScanEvent {
    FolderStarted(PathBuf),
    Progress(usize, usize),
    Done(Vec<Track>),
}

static IS_SCANNING: AtomicBool = AtomicBool::new(false);

/// Whether a background scan is currently holding the single-flight lock.
#[allow(dead_code)]
pub fn is_scanning() -> bool {
    IS_SCANNING.load(Ordering::SeqCst)
}

fn send_ev(tx: &Sender<ScanEvent>, ev: ScanEvent) {
    if tx.send(ev).is_err() {
        tracing::debug!("ScanEvent receiver dropped");
    }
}

fn run_scan_async_inner(paths: Vec<PathBuf>, tx: Sender<ScanEvent>) {
    let mut unique_roots: Vec<PathBuf> = Vec::new();
    let mut seen_roots: HashSet<PathBuf> = HashSet::new();

    for p in paths {
        let expanded = utils::expand_path(&p.to_string_lossy());
        let Ok(canon) = dunce::canonicalize(&expanded) else {
            tracing::warn!(path = %expanded.display(), "canonicalize failed during async scan — fix path in Library Manager");
            continue;
        };
        if !canon.is_dir() {
            tracing::warn!(path = %canon.display(), "not a directory; skipping");
            continue;
        }
        if seen_roots.insert(canon.clone()) {
            send_ev(&tx, ScanEvent::FolderStarted(canon.clone()));
            unique_roots.push(canon);
        }
    }

    let mut files: Vec<PathBuf> = Vec::new();
    for root in &unique_roots {
        walk_dir(root, &mut files);
    }

    let mut seen_files: HashSet<PathBuf> = HashSet::new();
    let mut unique_files: Vec<PathBuf> = Vec::new();
    for f in files {
        if let Ok(c) = dunce::canonicalize(&f) {
            if seen_files.insert(c.clone()) {
                unique_files.push(c);
            }
        }
    }

    unique_files.sort();
    let total = unique_files.len();
    let mut tracks = Vec::with_capacity(total);

    for (i, path) in unique_files.into_iter().enumerate() {
        tracks.push(extract_metadata(&path));
        if total > 0 {
            send_ev(&tx, ScanEvent::Progress(i + 1, total));
        }
    }

    send_ev(&tx, ScanEvent::Done(tracks));
}

/// Spawn a background scan. Returns `None` if another scan is already running (single-flight).
///
/// Drain `ScanEvent` on `tx`'s paired receiver; terminal event is [`ScanEvent::Done`].
#[allow(dead_code)]
pub fn scan_async(paths: Vec<PathBuf>, tx: Sender<ScanEvent>) -> Option<std::thread::JoinHandle<()>> {
    match IS_SCANNING.compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst) {
        Ok(_) => {}
        Err(_) => {
            tracing::warn!("rescan already in progress");
            return None;
        }
    }

    let tx = Arc::new(tx);
    Some(std::thread::spawn(move || {
        struct ClearFlag;
        impl Drop for ClearFlag {
            fn drop(&mut self) {
                IS_SCANNING.store(false, Ordering::SeqCst);
            }
        }
        let _clear = ClearFlag;
        run_scan_async_inner(paths, (*tx).clone());
    }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::time::Duration;

    #[test]
    fn clean_filename_strips_yt_dlp_brackets() {
        let stem = "Hans Zimmer - Time (Inception Soundtrack) [1 Hour] [j1M9tZGznx0]";
        let (a, t) = clean_filename_title(stem);
        assert_eq!(a.as_deref(), Some("Hans Zimmer"));
        assert_eq!(t, "Time (Inception Soundtrack)");
    }

    #[test]
    fn clean_filename_keeps_plain_title() {
        let (a, t) = clean_filename_title("weird");
        assert!(a.is_none());
        assert_eq!(t, "weird");
    }

    #[test]
    fn clean_filename_strips_topic_suffix() {
        let (a, t) = clean_filename_title("Artist - Topic - Song");
        assert_eq!(a.as_deref(), Some("Artist"));
        assert_eq!(t, "Song");
    }

    #[test]
    fn extension_filter() {
        assert!(is_audio_extension("mp3"));
        assert!(is_audio_extension("MP3"));
        assert!(!is_audio_extension("txt"));
        assert!(is_audio_extension("flac"));
    }

    #[test]
    fn scan_finds_files_and_dedupes() {
        let dir = std::env::temp_dir().join(format!("rtunes-scan-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        std::fs::File::create(dir.join("a.mp3")).unwrap().write_all(b"x").unwrap();
        std::fs::File::create(dir.join("b.flac")).unwrap().write_all(b"x").unwrap();
        std::fs::File::create(dir.join("sub/c.wav")).unwrap().write_all(b"x").unwrap();
        std::fs::File::create(dir.join("noise.txt")).unwrap().write_all(b"x").unwrap();

        let root = dunce::canonicalize(&dir).unwrap();
        let tracks = scan_paths(&[root.clone(), root]);
        assert_eq!(tracks.len(), 3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn metadata_fallback_for_unparseable() {
        let dir = std::env::temp_dir().join(format!("rtunes-badmp3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let p = dir.join("weird.mp3");
        std::fs::File::create(&p).unwrap().write_all(b"xxxx").unwrap();
        let root = dunce::canonicalize(&dir).unwrap();
        let tracks = scan_paths(&[root]);
        assert_eq!(tracks.len(), 1);
        assert_eq!(tracks[0].title, "weird");
        assert_eq!(tracks[0].duration_secs, 0);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn scan_async_event_sequence() {
        let dir = std::env::temp_dir().join(format!("rtunes-async-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::File::create(dir.join("x.mp3"))
            .unwrap()
            .write_all(b"x")
            .unwrap();
        let root = dunce::canonicalize(&dir).unwrap();

        let (tx, rx) = crossbeam_channel::unbounded();
        let handle = scan_async(vec![root], tx).expect("spawn scan");
        let mut events = Vec::new();
        while let Ok(ev) = rx.recv_timeout(Duration::from_secs(5)) {
            let done = matches!(ev, ScanEvent::Done(_));
            events.push(ev);
            if done {
                break;
            }
        }
        handle.join().unwrap();

        assert!(matches!(events.first(), Some(ScanEvent::FolderStarted(_))));
        assert!(events.iter().any(|e| matches!(e, ScanEvent::Progress(_, _))));
        assert!(matches!(events.last(), Some(ScanEvent::Done(t)) if !t.is_empty()));

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn single_flight_second_spawn_returns_none() {
        let dir = std::env::temp_dir().join(format!("rtunes-sf-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        for i in 0..50 {
            std::fs::File::create(dir.join(format!("f{i}.mp3")))
                .unwrap()
                .write_all(b"x")
                .unwrap();
        }
        let root = dunce::canonicalize(&dir).unwrap();

        let (tx1, _rx1) = crossbeam_channel::unbounded();
        let h1 = scan_async(vec![root.clone()], tx1).expect("first scan");

        let (tx2, rx2) = crossbeam_channel::unbounded();
        let h2 = scan_async(vec![root], tx2);
        assert!(h2.is_none());
        assert!(rx2.try_recv().is_err());

        let _ = h1.join();

        let _ = std::fs::remove_dir_all(&dir);
    }
}
