//! Per-frame data from the FFT thread to the renderer (Phase 6+).

use std::collections::VecDeque;
use std::sync::Arc;
use std::time::{Duration, Instant};

/// Per-frame data delivered from the FFT thread to the renderer.
///
/// The `spectrogram_rows` ring is wrapped in `Arc` so the renderer receives a
/// cheap pointer clone instead of a full deep copy (128 rows × 64 floats).
#[derive(Debug, Clone)]
pub struct VisualizerData {
    pub bins_raw: Vec<f32>,
    pub bins_smoothed: Vec<f32>,
    pub bins_peak: Vec<f32>,
    /// Previous frame's smoothed bins for sub-frame interpolation.
    pub bins_prev: Vec<f32>,
    pub pcm_mono: Vec<f32>,
    pub pcm_stereo: Vec<(f32, f32)>,
    pub bass_energy: f32,
    pub mid_energy: f32,
    pub high_energy: f32,
    pub loudness: f32,
    pub beat: bool,
    pub beat_intensity: f32,
    pub bpm_estimate: Option<f32>,
    /// Shared spectrogram ring — cheap to clone (Arc pointer).
    pub spectrogram_rows: Arc<VecDeque<Vec<f32>>>,
    pub timestamp: Instant,
    pub fft_period: Duration,
}

impl VisualizerData {
    pub fn empty(num_bins: usize) -> Self {
        let now = Instant::now();
        Self {
            bins_raw: vec![0.0; num_bins],
            bins_smoothed: vec![0.0; num_bins],
            bins_peak: vec![0.0; num_bins],
            bins_prev: vec![0.0; num_bins],
            pcm_mono: Vec::new(),
            pcm_stereo: Vec::new(),
            bass_energy: 0.0,
            mid_energy: 0.0,
            high_energy: 0.0,
            loudness: 0.0,
            beat: false,
            beat_intensity: 0.0,
            bpm_estimate: None,
            spectrogram_rows: Arc::new(VecDeque::new()),
            timestamp: now,
            fft_period: Duration::from_millis(33),
        }
    }
}
