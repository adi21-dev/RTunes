//! Native OS file/folder picker (rfd) for selecting yt-dlp and ffmpeg binaries.
//!
//! Async helpers spawn a background thread so the TUI render loop is never blocked.
//! Results arrive via a `crossbeam_channel::Sender<PickerEvent>`.
//!
//! Use the blocking variants (`*_blocking`) from CLI code only.

use std::path::{Path, PathBuf};

use crossbeam_channel::Sender;

/// Which binary is being configured.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickerTarget {
    YtDlp,
    Ffmpeg,
    /// The default output directory for downloads.
    DownloadDir,
}

/// Result of one picker session.
#[derive(Debug)]
pub struct PickerEvent {
    /// Which binary field this result belongs to.
    pub target: PickerTarget,
    /// Selected path, or `None` when the user cancelled the dialog.
    pub path: Option<PathBuf>,
}

/// Open a native **file** dialog for the given binary target (TUI-safe, non-blocking).
///
/// Spawns a thread and sends exactly one [`PickerEvent`] through `tx` when done.
pub fn open_binary_picker_async(
    target: PickerTarget,
    suggested_dir: Option<PathBuf>,
    tx: Sender<PickerEvent>,
) {
    std::thread::spawn(move || {
        let path = build_file_dialog(target, suggested_dir.as_deref()).pick_file();
        let _ = tx.send(PickerEvent { target, path });
    });
}

/// Open a native **folder** dialog (the binary will be searched inside the chosen directory).
///
/// Spawns a thread and sends exactly one [`PickerEvent`] through `tx` when done.
pub fn open_dir_picker_async(
    target: PickerTarget,
    suggested_dir: Option<PathBuf>,
    tx: Sender<PickerEvent>,
) {
    std::thread::spawn(move || {
        let dir = build_dir_dialog(target, suggested_dir.as_deref()).pick_folder();
        let _ = tx.send(PickerEvent { target, path: dir });
    });
}

/// Open a **blocking** file dialog (CLI use only — must not be called from the TUI loop).
pub fn pick_binary_blocking(target: PickerTarget, suggested_dir: Option<&Path>) -> Option<PathBuf> {
    build_file_dialog(target, suggested_dir).pick_file()
}

/// Open a **blocking** folder dialog (CLI use only).
pub fn pick_dir_blocking(target: PickerTarget, suggested_dir: Option<&Path>) -> Option<PathBuf> {
    build_dir_dialog(target, suggested_dir).pick_folder()
}

fn label_for(target: PickerTarget) -> &'static str {
    match target {
        PickerTarget::YtDlp => "yt-dlp",
        PickerTarget::Ffmpeg => "ffmpeg",
        PickerTarget::DownloadDir => "downloads",
    }
}

fn build_file_dialog(target: PickerTarget, suggested_dir: Option<&Path>) -> rfd::FileDialog {
    let label = label_for(target);
    let mut d = rfd::FileDialog::new().set_title(format!("Select {label} binary"));
    #[cfg(windows)]
    {
        d = d.add_filter("Executable", &["exe"]);
    }
    if let Some(dir) = suggested_dir {
        d = d.set_directory(dir);
    }
    d
}

fn build_dir_dialog(target: PickerTarget, suggested_dir: Option<&Path>) -> rfd::FileDialog {
    let label = label_for(target);
    let mut d = rfd::FileDialog::new()
        .set_title(format!("Select folder containing {label}"));
    if let Some(dir) = suggested_dir {
        d = d.set_directory(dir);
    }
    d
}
