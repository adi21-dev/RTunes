//! Path expansion, canonicalization, and binary resolution (PATH → exe-adjacent `deps/`).

use std::io;
use std::path::{Component, Path, PathBuf};

use crate::error::{Result, RtunesError};

/// Expands `~` and (via shellexpand) user home; falls back to the raw string on expand failure.
pub fn expand_path(input: &str) -> PathBuf {
    match shellexpand::tilde(input) {
        std::borrow::Cow::Borrowed(s) => PathBuf::from(s),
        std::borrow::Cow::Owned(s) => PathBuf::from(s),
    }
}

/// Canonical path without Windows `\\?\` UNC prefix when possible.
pub fn canonical(path: &Path) -> std::io::Result<PathBuf> {
    dunce::canonicalize(path)
}

/// Resolve `root.join(rel)` and ensure the result stays inside `root` (no `..` escape, no abs paths).
///
/// Used when combining a trusted base directory with user-relative segments.
pub fn safe_path(root: &Path, rel: &str) -> Result<PathBuf> {
    let rel_path = Path::new(rel);
    if rel_path.is_absolute() {
        return Err(RtunesError::Io(io::Error::other(
            "path must be relative to the trusted root",
        )));
    }
    if rel_path
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err(RtunesError::Io(io::Error::other(
            "path must not contain parent-dir (..) components",
        )));
    }

    let root_canon = canonical(root).map_err(|e| {
        RtunesError::Io(io::Error::other(format!(
            "cannot canonicalize trusted root {}: {e}",
            root.display()
        )))
    })?;

    let joined = root.join(rel_path);
    let resolved = joined.canonicalize().map_err(|e| {
        RtunesError::Io(io::Error::other(format!(
            "cannot resolve {} under {}: {e}. Check the path exists.",
            rel,
            root.display()
        )))
    })?;

    // Compare with `dunce::simplified` so Windows `\\?\` extended paths match normal prefixes.
    let root_cmp = dunce::simplified(&root_canon);
    let res_cmp = dunce::simplified(&resolved);
    if !AsRef::<Path>::as_ref(&res_cmp).starts_with(AsRef::<Path>::as_ref(&root_cmp)) {
        return Err(RtunesError::Io(io::Error::other(format!(
            "resolved path escapes trusted root: {}",
            resolved.display()
        ))));
    }

    Ok(resolved)
}

fn executable_name_candidates(name: &str) -> Vec<String> {
    #[cfg(windows)]
    {
        // Windows executables: prefer .exe, then .cmd (batch wrappers are common for
        // tools installed via package managers like scoop or winget), then bare name
        // (some tools ship without an extension even on Windows).
        let lower = name.to_lowercase();
        if lower.ends_with(".exe")
            || lower.ends_with(".cmd")
            || lower.ends_with(".bat")
            || lower.ends_with(".ps1")
        {
            vec![name.to_string()]
        } else {
            vec![format!("{name}.exe"), format!("{name}.cmd"), name.to_string()]
        }
    }
    #[cfg(target_os = "macos")]
    {
        // macOS: plain name first (Homebrew / renamed binary), then the `<name>_macos`
        // artifact that yt-dlp's releases page ships so users can drop it in deps/
        // without renaming.  No .app bundle or extension is used for CLI tools.
        vec![name.to_string(), format!("{name}_macos")]
    }
    #[cfg(not(any(windows, target_os = "macos")))]
    {
        // Linux (and other Unix): plain binary name only, no extension.
        // Distro packages (apt, dnf, pacman) and static builds all ship without extension.
        vec![name.to_string()]
    }
}

fn first_existing_in_dir(dir: &Path, candidates: &[String]) -> Option<PathBuf> {
    for c in candidates {
        let p = dir.join(c);
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

/// Search for a named binary inside `dir`, respecting platform executable name candidates.
///
/// Returns the first matching file path, or `None` if none exist.
pub fn find_binary_in_dir(dir: &Path, name: &str) -> Option<PathBuf> {
    let candidates = executable_name_candidates(name);
    first_existing_in_dir(dir, &candidates)
}

/// Resolves a binary: check `<current_exe>/deps/<name>` first, then `PATH`.
///
/// Checking the exe-adjacent `deps/` folder first means a bundled deployment
/// (e.g. a release archive with `deps/yt-dlp.exe`) works without any PATH setup.
pub fn resolve_binary(name: &str) -> Option<PathBuf> {
    let candidates = executable_name_candidates(name);

    // 1. Prefer the exe-adjacent deps/ folder (bundled deployment).
    if let Ok(exe) = std::env::current_exe() {
        if let Some(parent) = exe.parent() {
            let deps = parent.join("deps");
            if let Some(p) = first_existing_in_dir(&deps, &candidates) {
                return Some(p);
            }
        }
    }

    // 2. Fall back to PATH (system-wide install).
    if let Ok(paths) = std::env::var("PATH") {
        #[cfg(windows)]
        let sep = ';';
        #[cfg(not(windows))]
        let sep = ':';

        for dir in paths.split(sep) {
            if dir.is_empty() {
                continue;
            }
            if let Some(p) = first_existing_in_dir(Path::new(dir), &candidates) {
                return Some(p);
            }
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn safe_path_resolves_file_under_root() {
        let dir =
            std::env::temp_dir().join(format!("rtunes-safe-path-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("sub")).unwrap();
        let f = dir.join("sub").join("x.txt");
        std::fs::write(&f, b"hi").unwrap();
        let got = safe_path(&dir, "sub/x.txt").unwrap();
        let expect = dunce::canonicalize(&f).unwrap();
        let g = dunce::simplified(&got).to_path_buf();
        let e = dunce::simplified(&expect).to_path_buf();
        assert_eq!(g, e);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn safe_path_rejects_parent_dir() {
        let dir =
            std::env::temp_dir().join(format!("rtunes-safe-path2-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        assert!(safe_path(&dir, "../etc/passwd").is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn safe_path_rejects_absolute_rel() {
        let dir =
            std::env::temp_dir().join(format!("rtunes-safe-path3-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        #[cfg(unix)]
        assert!(safe_path(&dir, "/etc/passwd").is_err());
        #[cfg(windows)]
        {
            let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into());
            let p = format!("{windir}\\notepad.exe");
            assert!(safe_path(&dir, &p).is_err());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn safe_path_rejects_symlink_outside_root() {
        use std::os::unix::fs::symlink;

        let dir =
            std::env::temp_dir().join(format!("rtunes-safe-sym-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let outside = std::env::temp_dir().join(format!("rtunes-safe-out-{}", std::process::id()));
        let _ = std::fs::remove_file(&outside);
        std::fs::write(&outside, b"x").unwrap();
        let link = dir.join("leak");
        let _ = std::fs::remove_file(&link);
        symlink(&outside, &link).unwrap();
        assert!(safe_path(&dir, "leak").is_err());
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_file(&outside);
    }
}
