//! Runtime auto-download of yt-dlp and ffmpeg into the `deps/` folder beside the executable.
//!
//! When a user attempts a download and the required tools are not found, rtunes offers to
//! download them automatically. They are saved to `<exe_dir>/deps/` which is the first
//! location checked by [`crate::utils::resolve_binary`], so they are used immediately on
//! the next attempt without any config change.
//!
//! **macOS note**: ffmpeg is not auto-downloaded on macOS because no reliable single-file
//! static build is available. Users are directed to `brew install ffmpeg` instead.

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

/// Returns the `deps/` directory adjacent to the running executable, creating it if needed.
///
/// Returns `None` only if `current_exe()` fails (unusual in practice).
pub fn deps_dir() -> Option<PathBuf> {
    let dir = std::env::current_exe()
        .ok()?
        .parent()
        .map(|p| p.join("deps"))?;
    fs::create_dir_all(&dir).ok()?;
    Some(dir)
}

/// Whether ffmpeg can be auto-downloaded on this platform.
///
/// Returns `false` on macOS where no static single-binary release exists; users
/// should install via `brew install ffmpeg`.
pub fn ffmpeg_auto_download_supported() -> bool {
    !cfg!(target_os = "macos")
}

/// Instructions to install ffmpeg manually on the current platform.
pub fn ffmpeg_manual_instructions() -> &'static str {
    #[cfg(target_os = "macos")]
    return "Install ffmpeg via Homebrew: brew install ffmpeg";
    #[cfg(windows)]
    return "Install ffmpeg via winget: winget install Gyan.FFmpeg";
    #[cfg(not(any(target_os = "macos", windows)))]
    return "Install ffmpeg via your package manager (e.g. sudo apt install ffmpeg)";
}

/// Platform-specific yt-dlp GitHub release asset name.
fn ytdlp_asset_name() -> &'static str {
    #[cfg(windows)]
    return "yt-dlp.exe";
    #[cfg(target_os = "macos")]
    return "yt-dlp_macos";
    #[cfg(all(not(windows), not(target_os = "macos"), target_arch = "aarch64"))]
    return "yt-dlp_linux_aarch64";
    #[cfg(all(not(windows), not(target_os = "macos"), not(target_arch = "aarch64")))]
    return "yt-dlp_linux";
}

/// Filename used when saving yt-dlp into `deps/`.
fn ytdlp_dest_filename() -> &'static str {
    #[cfg(windows)]
    return "yt-dlp.exe";
    #[cfg(not(windows))]
    return "yt-dlp";
}

/// BtbN ffmpeg archive asset name for the current platform.
/// Returns `None` on macOS (auto-download not supported).
fn ffmpeg_btbn_asset() -> Option<&'static str> {
    #[cfg(windows)]
    return Some("win64");
    #[cfg(all(not(windows), not(target_os = "macos"), target_arch = "aarch64"))]
    return Some("linuxarm64");
    #[cfg(all(not(windows), not(target_os = "macos"), not(target_arch = "aarch64")))]
    return Some("linux64");
    #[cfg(target_os = "macos")]
    return None;
}

/// Filename used when saving ffmpeg into `deps/`.
fn ffmpeg_dest_filename() -> &'static str {
    #[cfg(windows)]
    return "ffmpeg.exe";
    #[cfg(not(windows))]
    return "ffmpeg";
}

/// Download a URL to `dest`, reporting progress in [0.0, 1.0] via `on_progress`.
///
/// Downloads to a `.tmp` sibling file first, then renames atomically (best-effort on Windows).
/// Sets `+x` permission on Unix after the rename.
fn download_to_file<F>(url: &str, dest: &Path, on_progress: &F) -> Result<(), String>
where
    F: Fn(f32),
{
    let response = ureq::get(url)
        .call()
        .map_err(|e| format!("HTTP request to {url} failed: {e}"))?;

    let content_length: Option<u64> = response
        .header("Content-Length")
        .and_then(|v| v.parse().ok());

    let mut reader = response.into_reader();

    // Write to a temp path so a failed download doesn't leave a corrupt file.
    let tmp = dest.with_extension("_dl_tmp");
    {
        let mut file =
            fs::File::create(&tmp).map_err(|e| format!("create {}: {e}", tmp.display()))?;

        let mut buf = [0u8; 65536];
        let mut downloaded: u64 = 0;
        loop {
            let n = reader
                .read(&mut buf)
                .map_err(|e| format!("read from {url}: {e}"))?;
            if n == 0 {
                break;
            }
            file.write_all(&buf[..n])
                .map_err(|e| format!("write to {}: {e}", tmp.display()))?;
            downloaded += n as u64;
            if let Some(total) = content_length {
                on_progress((downloaded as f32 / total as f32).clamp(0.0, 0.99));
            }
        }
        file.sync_all()
            .map_err(|e| format!("sync {}: {e}", tmp.display()))?;
    }

    // Rename into place.
    fs::rename(&tmp, dest)
        .map_err(|e| format!("rename {} -> {}: {e}", tmp.display(), dest.display()))?;

    // Set executable permission on Unix.
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let mut perms = fs::metadata(dest).map_err(|e| e.to_string())?.permissions();
        perms.set_mode(0o755);
        fs::set_permissions(dest, perms).map_err(|e| e.to_string())?;
    }

    on_progress(1.0);
    Ok(())
}

/// Download yt-dlp into `dest_dir`, saving it as `yt-dlp[.exe]`.
///
/// `on_progress` is called with values in [0.0, 1.0] as bytes arrive.
/// Returns the full path to the downloaded binary on success.
pub fn download_ytdlp<F>(dest_dir: &Path, on_progress: F) -> Result<PathBuf, String>
where
    F: Fn(f32),
{
    let asset = ytdlp_asset_name();
    let url = format!("https://github.com/yt-dlp/yt-dlp/releases/latest/download/{asset}");
    let dest = dest_dir.join(ytdlp_dest_filename());
    tracing::info!(%url, dest = %dest.display(), "downloading yt-dlp");
    download_to_file(&url, &dest, &on_progress)?;
    tracing::info!(dest = %dest.display(), "yt-dlp download complete");
    Ok(dest)
}

/// Download ffmpeg into `dest_dir`, saving it as `ffmpeg[.exe]`.
///
/// Returns `Err` on macOS (use [`ffmpeg_manual_instructions`] instead).
/// `on_progress` is called with values in [0.0, 1.0] as bytes arrive.
/// Returns the full path to the downloaded binary on success.
pub fn download_ffmpeg<F>(dest_dir: &Path, on_progress: F) -> Result<PathBuf, String>
where
    F: Fn(f32),
{
    let asset = ffmpeg_btbn_asset().ok_or_else(|| ffmpeg_manual_instructions().to_string())?;

    let dest = dest_dir.join(ffmpeg_dest_filename());

    #[cfg(windows)]
    {
        let _ = asset; // used below in the URL
        let url = "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/ffmpeg-master-latest-win64-gpl.zip";
        let tmp_dir = std::env::temp_dir().join(format!("rtunes-ffmpeg-{}", std::process::id()));
        let zip_path = tmp_dir.join("ffmpeg.zip");
        fs::create_dir_all(&tmp_dir).map_err(|e| format!("create tmp dir: {e}"))?;

        tracing::info!(%url, "downloading ffmpeg archive (Windows)");
        // Split progress: 0–70% for archive download, 70–100% for extraction.
        download_to_file(url, &zip_path, &|p| on_progress(p * 0.70))?;

        // Extract using PowerShell (always available on Windows).
        let extract_dir = tmp_dir.join("extracted");
        let status = std::process::Command::new("powershell")
            .args([
                "-NoProfile",
                "-Command",
                &format!(
                    "Expand-Archive -Path '{}' -DestinationPath '{}' -Force",
                    zip_path.display(),
                    extract_dir.display()
                ),
            ])
            .status()
            .map_err(|e| format!("launch PowerShell: {e}"))?;
        if !status.success() {
            return Err(format!("Expand-Archive failed with {status}"));
        }
        on_progress(0.85);

        // Find ffmpeg.exe inside the extracted tree (under …/bin/ffmpeg.exe).
        let ffmpeg_exe = find_file_recursive(&extract_dir, "ffmpeg.exe")
            .ok_or_else(|| "ffmpeg.exe not found in extracted archive".to_string())?;
        fs::copy(&ffmpeg_exe, &dest).map_err(|e| format!("copy ffmpeg.exe: {e}"))?;

        // Clean up temp dir (best-effort).
        let _ = fs::remove_dir_all(&tmp_dir);
        on_progress(1.0);
    }

    #[cfg(not(windows))]
    {
        let archive_name = format!("ffmpeg-master-latest-{asset}-gpl.tar.xz");
        let url = format!(
            "https://github.com/BtbN/FFmpeg-Builds/releases/download/latest/{archive_name}"
        );
        let tmp_dir = std::env::temp_dir().join(format!("rtunes-ffmpeg-{}", std::process::id()));
        let archive_path = tmp_dir.join(&archive_name);
        fs::create_dir_all(&tmp_dir).map_err(|e| format!("create tmp dir: {e}"))?;

        tracing::info!(%url, "downloading ffmpeg archive (Linux)");
        download_to_file(&url, &archive_path, &|p| on_progress(p * 0.70))?;

        // Extract using system tar (always present on Linux).
        let status = std::process::Command::new("tar")
            .arg("-xJf")
            .arg(&archive_path)
            .arg("-C")
            .arg(&tmp_dir)
            .status()
            .map_err(|e| format!("tar: {e}"))?;
        if !status.success() {
            return Err(format!("tar exited with {status}"));
        }
        on_progress(0.85);

        // Find the ffmpeg binary inside the extracted tree.
        let ffmpeg_bin = find_file_recursive(&tmp_dir, "ffmpeg")
            .ok_or_else(|| "ffmpeg binary not found in extracted archive".to_string())?;
        fs::copy(&ffmpeg_bin, &dest).map_err(|e| format!("copy ffmpeg: {e}"))?;

        // chmod +x
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = fs::metadata(&dest)
                .map_err(|e| e.to_string())?
                .permissions();
            perms.set_mode(0o755);
            fs::set_permissions(&dest, perms).map_err(|e| e.to_string())?;
        }

        let _ = fs::remove_dir_all(&tmp_dir);
        on_progress(1.0);
    }

    tracing::info!(dest = %dest.display(), "ffmpeg download complete");
    Ok(dest)
}

/// Recursively find the first file with `name` inside `dir`.
fn find_file_recursive(dir: &Path, name: &str) -> Option<PathBuf> {
    let rd = fs::read_dir(dir).ok()?;
    for entry in rd.flatten() {
        let path = entry.path();
        if path.is_dir() {
            if let Some(found) = find_file_recursive(&path, name) {
                return Some(found);
            }
        } else if path.file_name().and_then(|n| n.to_str()) == Some(name) {
            return Some(path);
        }
    }
    None
}
