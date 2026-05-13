//! yt-dlp-backed downloads, bounded concurrency, and URL validation.
//!
//! Typical usage from the TUI or CLI:
//! 1. [`validate_url`] on user input.
//! 2. Build a [`FetcherPool`] with a [`YtDlpFetcher`] (or [`MockFetcher`] in tests).
//! 3. Call [`Fetcher::fetch`] on a worker thread and forward [`FetchEvent`] values to the UI.

mod downloader;
pub mod deps;
pub mod picker;
pub mod pool;

pub use downloader::{
    resolve_fetcher_tool, try_resolve_tools, validate_url, FetchEvent, FetchOpts, Fetcher,
    MissingTool, MockFetcher, YtDlpFetcher,
};
pub use deps::{deps_dir, download_ffmpeg, download_ytdlp, ffmpeg_auto_download_supported, ffmpeg_manual_instructions};
pub use picker::{
    open_binary_picker_async, open_dir_picker_async, pick_binary_blocking, pick_dir_blocking,
    PickerEvent, PickerTarget,
};
pub use pool::FetcherPool;
