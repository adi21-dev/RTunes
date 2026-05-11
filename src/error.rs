//! Custom error types for RTunes.

#[derive(Debug, thiserror::Error)]
pub enum RtunesError {
    #[error("Config error: {0}")]
    Config(String),

    #[error("Audio playback error: {0}")]
    Audio(String),

    #[error("FFT processing error: {0}")]
    Visualizer(String),

    #[error("Download failed: {0}")]
    Fetcher(String),

    #[error("IO error: {0}")]
    Io(#[from] std::io::Error),
}

pub type Result<T> = std::result::Result<T, RtunesError>;
