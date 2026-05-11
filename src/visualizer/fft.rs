//! Hann window, forward FFT, log-frequency binning, per-frame peak-relative dB mapping, band energies.

use std::f32::consts::PI;
use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

pub const EPSILON: f32 = 1.0e-9;

/// Relative dB window: `rel_db = band_db - peak_db`; `-D` below peak maps to 0; peak maps to 1.
const REL_DYNAMIC_DB: f32 = 34.0;
/// Companding exponent (`< 1` lifts quiet bands).
const REL_COMPAND: f32 = 0.85;
/// Frames whose spectral peak falls below this dB render as silence.
const REL_SILENCE_DB: f32 = -65.0;
/// Blend toward full A-weighting (0 = off, 1 = full).
const A_WEIGHT_BLEND: f32 = 0.62;

#[inline]
fn a_weight(freq_hz: f32) -> f32 {
    const F_LOW: f32 = 107.7;
    const F_HIGH: f32 = 737.9;
    let f = freq_hz.max(1.0);
    let lo = 1.0 / (1.0 + (F_LOW / f).powi(2));
    let hi = 1.0 / (1.0 + (f / F_HIGH).powi(2));
    (lo * hi).clamp(0.0, 1.0)
}

/// Per-frame peak-relative dB to `[0, 1]` (Python `relative` live-normalize path).
pub(crate) fn linear_bands_to_relative_bins(
    band_linear: &[f32],
    band_db_scratch: &mut [f32],
    out_bins: &mut [f32],
) {
    let n = band_linear
        .len()
        .min(band_db_scratch.len())
        .min(out_bins.len());
    if n == 0 {
        return;
    }
    for i in 0..n {
        band_db_scratch[i] = 20.0 * band_linear[i].max(EPSILON).log10();
    }
    let mut peak_db = f32::NEG_INFINITY;
    for i in 0..n {
        peak_db = peak_db.max(band_db_scratch[i]);
    }
    if peak_db <= REL_SILENCE_DB {
        out_bins[..n].fill(0.0);
        return;
    }
    for i in 0..n {
        let rel = band_db_scratch[i] - peak_db;
        let t = ((rel + REL_DYNAMIC_DB) / REL_DYNAMIC_DB).clamp(0.0, 1.0);
        out_bins[i] = t.powf(REL_COMPAND);
    }
}

/// Apply Hann window in place (matches implementation spec).
pub fn apply_hann_window(samples: &mut [f32]) {
    let n = samples.len();
    if n == 0 {
        return;
    }
    for (i, sample) in samples.iter_mut().enumerate() {
        let w = 0.5 * (1.0 - f32::cos(2.0 * PI * i as f32 / n as f32));
        *sample *= w;
    }
}

fn log_edges(num_bins: usize, sample_rate: u32) -> Vec<f32> {
    let nyquist = (sample_rate as f32) * 0.5;
    let f_max = nyquist.min(20_000.0).max(40.0);
    let f_min = 20.0f32;
    let mut edges = Vec::with_capacity(num_bins + 1);
    let log_min = f_min.ln();
    let log_max = f_max.ln();
    for i in 0..=num_bins {
        let t = i as f32 / num_bins as f32;
        let f = (log_min + t * (log_max - log_min)).exp();
        edges.push(f);
    }
    edges
}

/// FFT + log binning for visualization.
pub struct FftCore {
    fft: Arc<dyn Fft<f32>>,
    window: Vec<f32>,
    scratch: Vec<Complex<f32>>,
    /// `edges.len() == num_bins + 1`, Hz boundaries (log-spaced 20 Hz .. min(20k, Nyquist)).
    pub log_bin_edges: Vec<f32>,
    num_bins: usize,
    window_size: usize,
    sample_rate: u32,
    /// Linear magnitudes per FFT bin before log aggregation (reused).
    linear_fft: Vec<f32>,
    /// Per-bin linear band means before dB / relative mapping.
    band_linear: Vec<f32>,
    band_db: Vec<f32>,
    /// A-weight blend per FFT bin index `k` (multiply `|X[k]|` before log-band pooling).
    a_weights: Vec<f32>,
}

impl FftCore {
    pub fn new(window_size: usize, num_bins: usize, sample_rate: u32) -> Self {
        assert!(window_size.is_power_of_two());
        let mut planner = FftPlanner::new();
        let fft = planner.plan_fft_forward(window_size);
        let window: Vec<f32> = (0..window_size)
            .map(|i| 0.5 * (1.0 - f32::cos(2.0 * PI * i as f32 / window_size as f32)))
            .collect();
        let sr = sample_rate.max(1);
        let edges = log_edges(num_bins, sr);
        let half = window_size / 2;
        let mut a_weights = vec![1.0f32; half];
        for k in 1..half {
            let fk = k as f32 * sr as f32 / window_size as f32;
            a_weights[k] = a_weight(fk).powf(A_WEIGHT_BLEND);
        }
        Self {
            fft,
            window,
            scratch: vec![Complex::new(0.0, 0.0); window_size],
            log_bin_edges: edges,
            num_bins,
            window_size,
            sample_rate: sr,
            linear_fft: vec![0.0; half],
            band_linear: vec![0.0f32; num_bins],
            band_db: vec![0.0f32; num_bins],
            a_weights,
        }
    }

    pub fn rebuild_for_sample_rate(&mut self, sample_rate: u32) {
        let sr = sample_rate.max(1);
        if sr == self.sample_rate && !self.log_bin_edges.is_empty() {
            return;
        }
        self.sample_rate = sr;
        self.log_bin_edges = log_edges(self.num_bins, sr);
        let half = self.window_size / 2;
        if self.a_weights.len() != half {
            self.a_weights.resize(half, 1.0);
        }
        for k in 1..half {
            let fk = k as f32 * sr as f32 / self.window_size as f32;
            self.a_weights[k] = a_weight(fk).powf(A_WEIGHT_BLEND);
        }
    }

    pub fn window_size(&self) -> usize {
        self.window_size
    }

    pub fn num_bins(&self) -> usize {
        self.num_bins
    }

    /// Hann multiply, forward FFT, log-bin aggregate into `out_bins` (length `num_bins`).
    pub fn process(&mut self, mono_window: &mut [f32], out_bins: &mut [f32]) {
        debug_assert_eq!(mono_window.len(), self.window_size);
        debug_assert_eq!(out_bins.len(), self.num_bins);
        let n = self.window_size;
        let sr = self.sample_rate.max(1) as f32;

        for i in 0..n {
            self.scratch[i] = Complex::new(mono_window[i] * self.window[i], 0.0);
        }
        self.fft.process(&mut self.scratch);

        let half = n / 2;
        for k in 1..half {
            let c = self.scratch[k];
            let m = (c.re * c.re + c.im * c.im).sqrt();
            self.linear_fft[k] = m * self.a_weights[k];
        }
        self.linear_fft[0] = 0.0;

        let edges = &self.log_bin_edges;
        for j in 0..self.num_bins {
            let f_lo = edges[j];
            let f_hi = edges[j + 1];
            let mut sum = 0.0f32;
            let mut count = 0u32;
            for k in 1..half {
                let fk = k as f32 * sr / n as f32;
                if fk >= f_lo && fk < f_hi {
                    sum += self.linear_fft[k];
                    count += 1;
                }
            }
            let mag = if count > 0 {
                sum / count as f32
            } else {
                0.0
            };
            self.band_linear[j] = mag;
        }
        linear_bands_to_relative_bins(&self.band_linear, &mut self.band_db, out_bins);
    }

    /// Band averages using geometric center of each log band vs 20–250 / 250–4k / 4k+ Hz.
    pub fn band_energies(bins: &[f32], edges: &[f32]) -> (f32, f32, f32) {
        if bins.is_empty() || edges.len() != bins.len() + 1 {
            return (0.0, 0.0, 0.0);
        }
        let mut bass_s = 0.0f32;
        let mut bass_c = 0u32;
        let mut mid_s = 0.0f32;
        let mut mid_c = 0u32;
        let mut high_s = 0.0f32;
        let mut high_c = 0u32;
        for i in 0..bins.len() {
            let c = (edges[i] * edges[i + 1]).sqrt();
            if c >= 20.0 && c < 250.0 {
                bass_s += bins[i];
                bass_c += 1;
            } else if c >= 250.0 && c < 4000.0 {
                mid_s += bins[i];
                mid_c += 1;
            } else if c >= 4000.0 {
                high_s += bins[i];
                high_c += 1;
            }
        }
        let bass = if bass_c > 0 {
            bass_s / bass_c as f32
        } else {
            0.0
        };
        let mid = if mid_c > 0 {
            mid_s / mid_c as f32
        } else {
            0.0
        };
        let high = if high_c > 0 {
            high_s / high_c as f32
        } else {
            0.0
        };
        (bass, mid, high)
    }

    /// RMS loudness in `[0, 1]` (boosted so full-scale sine maps near 1).
    pub fn loudness(samples: &[f32]) -> f32 {
        if samples.is_empty() {
            return 0.0;
        }
        let mut s = 0.0f32;
        for &x in samples {
            s += x * x;
        }
        let rms = (s / samples.len() as f32).sqrt();
        (rms * std::f32::consts::SQRT_2).min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hann_window_endpoints_near_zero() {
        let mut v = vec![1.0f32; 4096];
        apply_hann_window(&mut v);
        assert!(v[0].abs() < 1e-5);
        assert!(v[4095].abs() < 1e-5);
        assert!((v[2048] - 1.0).abs() < 0.01);
    }

    fn sine_window(freq_hz: f32, sr: u32, n: usize) -> Vec<f32> {
        (0..n)
            .map(|i| {
                let t = i as f32 / sr as f32;
                (2.0 * PI * freq_hz * t).sin()
            })
            .collect()
    }

    #[test]
    fn peak_bin_440hz_at_48k() {
        let sr = 48_000u32;
        let n = 4096;
        let mut core = FftCore::new(n, 64, sr);
        let mut buf = sine_window(440.0, sr, n);
        let mut out = vec![0.0f32; 64];
        core.process(&mut buf, &mut out);
        let argmax = out
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        let edges = log_edges(64, sr);
        let c440 = (edges[argmax] * edges[argmax + 1]).sqrt();
        assert!(
            (c440 - 440.0).abs() < 120.0,
            "peak bin center {c440} Hz, idx {argmax}"
        );
    }

    #[test]
    fn peak_bin_880hz_shifts_up() {
        let sr = 48_000u32;
        let n = 4096;
        let mut core = FftCore::new(n, 64, sr);
        let mut buf440 = sine_window(440.0, sr, n);
        let mut out440 = vec![0.0f32; 64];
        core.process(&mut buf440, &mut out440);
        let i440 = out440
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .unwrap();

        let mut buf880 = sine_window(880.0, sr, n);
        let mut out880 = vec![0.0f32; 64];
        core.process(&mut buf880, &mut out880);
        let i880 = out880
            .iter()
            .enumerate()
            .max_by(|(_, a), (_, b)| a.partial_cmp(b).unwrap())
            .map(|(i, _)| i)
            .unwrap();
        assert!(i880 > i440, "880 peak idx {i880} vs 440 idx {i440}");
    }

    #[test]
    fn loudness_silence_zero_full_scale_square() {
        let silence = vec![0.0f32; 1024];
        assert!(FftCore::loudness(&silence) < 1e-6);
        let sq: Vec<f32> = (0..1024).map(|i| if i % 2 == 0 { 1.0 } else { -1.0 }).collect();
        assert!(FftCore::loudness(&sq) > 0.95);
    }

    #[test]
    fn fft_per_frame_normalization_makes_peak_band_reach_one() {
        let mut linear = vec![0.001f32; 64];
        linear[30] = 0.5;
        let mut db = vec![0.0f32; 64];
        let mut out = vec![0.0f32; 64];
        linear_bands_to_relative_bins(&linear, &mut db, &mut out);
        assert!(
            (out[30] - 1.0).abs() < 0.02,
            "peak bin should map near 1.0, got {}",
            out[30]
        );
        let quiet_max = out
            .iter()
            .enumerate()
            .filter(|(i, _)| *i != 30)
            .map(|(_, v)| *v)
            .fold(0.0f32, f32::max);
        assert!(
            quiet_max < 0.05,
            "quiet bins should stay low, max={quiet_max}"
        );
    }

    #[test]
    fn fft_silence_floor_collapses_to_zero() {
        let linear = vec![1e-7f32; 64];
        let mut db = vec![0.0f32; 64];
        let mut out = vec![0.0f32; 64];
        linear_bands_to_relative_bins(&linear, &mut db, &mut out);
        assert!(
            out.iter().all(|&x| x == 0.0),
            "expected all zeros, got {:?}",
            out
        );
    }
}
