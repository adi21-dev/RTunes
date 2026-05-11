//! Universal smoothing primitives for the visualizer pipeline.

use std::collections::VecDeque;
use std::time::{Duration, Instant};

/// Default asymmetric EMA attack coefficient (fast rise).
pub const DEFAULT_ATTACK: f32 = 0.5;
/// Default asymmetric EMA release coefficient (slow fall).
pub const DEFAULT_RELEASE: f32 = 0.15;

/// Display EMA for bar heights — smoother motion than raw FFT EMA defaults.
pub const DISPLAY_ATTACK: f32 = 0.50;
pub const DISPLAY_RELEASE: f32 = 0.08;

#[inline]
fn lerp(a: f32, b: f32, t: f32) -> f32 {
    a * (1.0 - t) + b * t
}

/// 3-tap weighted average `[0.25, 0.5, 0.25]` across bins (in-place).
///
/// Allocates a temporary buffer internally. When smoothing multiple slices per
/// frame, prefer [`apply_spectral_smoothing_with_scratch`] to reuse the buffer.
pub fn apply_spectral_smoothing(bins: &mut [f32]) {
    let n = bins.len();
    if n < 3 {
        return;
    }
    let mut tmp = vec![0.0f32; n];
    tmp[0] = bins[0];
    tmp[n - 1] = bins[n - 1];
    for i in 1..n - 1 {
        tmp[i] = 0.25 * bins[i - 1] + 0.5 * bins[i] + 0.25 * bins[i + 1];
    }
    bins.copy_from_slice(&tmp);
}

/// Same as [`apply_spectral_smoothing`] but reuses a caller-provided scratch buffer,
/// avoiding a heap allocation. `scratch` is resized if needed.
#[inline]
pub fn apply_spectral_smoothing_with_scratch(bins: &mut [f32], scratch: &mut Vec<f32>) {
    let n = bins.len();
    if n < 3 {
        return;
    }
    scratch.resize(n, 0.0);
    scratch[0] = bins[0];
    scratch[n - 1] = bins[n - 1];
    for i in 1..n - 1 {
        scratch[i] = 0.25 * bins[i - 1] + 0.5 * bins[i] + 0.25 * bins[i + 1];
    }
    bins.copy_from_slice(scratch);
}

/// Per-bin asymmetric EMA: faster attack than release.
pub fn asymmetric_ema(prev: f32, new: f32, attack: f32, release: f32) -> f32 {
    let t = if new > prev { attack } else { release };
    lerp(prev, new, t)
}

/// Frame-rate-independent EMA: alpha = 1 − exp(−dt / tau).
///
/// `tau` is the time constant in seconds (e.g. 0.05 = 50 ms half-life rise).
/// Works correctly regardless of FPS — call with actual `dt = 1.0 / fps`.
#[inline]
pub fn ema_dt(prev: f32, target: f32, tau: f32, dt: f32) -> f32 {
    let alpha = 1.0 - (-dt / tau.max(1e-6)).exp();
    lerp(prev, target, alpha.clamp(0.0, 1.0))
}

/// Peak hold with exponential decay toward the smoothed envelope.
pub fn peak_hold_drift(peak: f32, smoothed: f32, decay: f32) -> f32 {
    smoothed.max(peak * decay)
}

/// Lift high-frequency bins so treble is visible against natural FFT roll-off.
pub fn treble_tilt(bins: &mut [f32], strength: f32) {
    let n = bins.len();
    if n <= 1 {
        return;
    }
    let denom = (n - 1) as f32;
    for (i, b) in bins.iter_mut().enumerate() {
        let t = i as f32 / denom;
        let m = 1.0 + strength * t;
        *b = (*b * m).clamp(0.0, 1.0);
    }
}

/// Render-side auto-gain: scale quiet spectra toward `target` peak height (slow rise, fast fall).
pub struct AutoGain {
    gain: f32,
    target: f32,
    /// Lerp factor when reducing gain (loud transient).
    fast: f32,
    /// Lerp factor when increasing gain (quiet passage).
    slow: f32,
    min: f32,
    max: f32,
}

impl AutoGain {
    pub fn new() -> Self {
        Self {
            gain: 1.0,
            target: 0.85,
            fast: 0.12,
            slow: 0.03,
            min: 0.50,
            max: 4.0,
        }
    }

    pub fn with_limits(target: f32, min: f32, max: f32) -> Self {
        Self {
            gain: 1.0,
            target,
            fast: 0.20,
            slow: 0.04,
            min,
            max,
        }
    }

    #[inline]
    pub fn gain(&self) -> f32 {
        self.gain
    }

    /// Updates internal gain from current slice peak; returns gain to multiply magnitudes.
    pub fn apply(&mut self, slice: &[f32]) -> f32 {
        let peak = slice.iter().copied().fold(0.0f32, f32::max);
        let desired = (self.target / peak.max(1e-3)).clamp(self.min, self.max);
        // Slow when increasing gain (amplify), fast when decreasing (attenuate).
        let t = if desired > self.gain {
            self.slow
        } else {
            self.fast
        };
        self.gain = lerp(self.gain, desired, t);
        self.gain
    }

    pub fn reset(&mut self) {
        self.gain = 1.0;
    }
}

/// One Euro filter — adaptive low-pass for jittery scalar parameters.
pub struct OneEuroFilter {
    mincutoff: f32,
    beta: f32,
    dcutoff: f32,
    x_prev: Option<f32>,
    dx_prev: f32,
    last_t: Option<Instant>,
}

impl OneEuroFilter {
    pub fn new(freq: f32, mincutoff: f32, beta: f32) -> Self {
        Self {
            mincutoff,
            beta,
            dcutoff: 1.0 / (2.0 * std::f32::consts::PI * freq.max(1.0)),
            x_prev: None,
            dx_prev: 0.0,
            last_t: None,
        }
    }

    #[inline]
    fn alpha(cutoff: f32, dt: f32) -> f32 {
        let tau = 1.0 / (2.0 * std::f32::consts::PI * cutoff.max(1e-6));
        1.0 / (1.0 + tau / dt.max(1e-6))
    }

    pub fn filter(&mut self, x: f32, now: Instant) -> f32 {
        let dt = self
            .last_t
            .map(|t| (now - t).as_secs_f32().max(1e-4))
            .unwrap_or(0.016);
        self.last_t = Some(now);

        let dx = match self.x_prev {
            Some(px) => (x - px) / dt,
            None => 0.0,
        };
        let edx = {
            let a = Self::alpha(1.0 / self.dcutoff, dt);
            lerp(self.dx_prev, dx, a)
        };
        self.dx_prev = edx;

        let cutoff = self.mincutoff + self.beta * edx.abs();
        let a = Self::alpha(cutoff, dt);
        let out = match self.x_prev {
            Some(px) => lerp(px, x, a),
            None => x,
        };
        self.x_prev = Some(out);
        out
    }
}

/// Spectral-flux onset detector with adaptive threshold.
pub struct SpectralFluxBeatDetector {
    history: VecDeque<f32>,
    prev_mag: Vec<f32>,
    last_beat: Option<Instant>,
    intensity: f32,
    beat_times: VecDeque<Instant>,
    history_len: usize,
}

impl SpectralFluxBeatDetector {
    pub fn new(history_len: usize) -> Self {
        Self {
            history: VecDeque::with_capacity(history_len),
            prev_mag: Vec::new(),
            last_beat: None,
            intensity: 0.0,
            beat_times: VecDeque::with_capacity(16),
            history_len,
        }
    }

    /// Returns `(beat_this_frame, intensity)` where intensity decays after a beat.
    pub fn ingest(&mut self, mags: &[f32], now: Instant) -> (bool, f32) {
        if self.prev_mag.len() != mags.len() {
            self.prev_mag.resize(mags.len(), 0.0);
        }

        let mut flux = 0.0f32;
        for i in 0..mags.len() {
            let d = mags[i] - self.prev_mag[i];
            flux += d.max(0.0);
            self.prev_mag[i] = mags[i];
        }

        while self.history.len() >= self.history_len {
            self.history.pop_front();
        }
        self.history.push_back(flux);

        let mean_flux: f32 = if self.history.is_empty() {
            0.0
        } else {
            self.history.iter().copied().sum::<f32>() / self.history.len() as f32
        };
        let threshold = mean_flux * 1.5;

        let mut beat = false;
        if flux > threshold {
            let ok_interval = match self.last_beat {
                None => true,
                Some(t) => now.duration_since(t) >= Duration::from_millis(250),
            };
            if ok_interval {
                beat = true;
                self.last_beat = Some(now);
                self.intensity = 1.0;
                while self.beat_times.len() > 16 {
                    self.beat_times.pop_front();
                }
                self.beat_times.push_back(now);
            }
        }

        if !beat {
            self.intensity *= 0.92;
        }

        (beat, self.intensity)
    }

    /// Median of last inter-onset intervals (seconds), converted to BPM; needs ≥4 beats.
    pub fn bpm_estimate(&self) -> Option<f32> {
        if self.beat_times.len() < 5 {
            return None;
        }
        let mut ivals: Vec<f32> = self
            .beat_times
            .iter()
            .zip(self.beat_times.iter().skip(1))
            .map(|(a, b)| b.duration_since(*a).as_secs_f32())
            .collect();
        if ivals.len() < 4 {
            return None;
        }
        let take = ivals.len().min(8);
        let start = ivals.len().saturating_sub(take);
        ivals = ivals[start..].to_vec();
        ivals.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
        let mid = ivals.len() / 2;
        let median_dt = if ivals.len() % 2 == 0 {
            (ivals[mid - 1] + ivals[mid]) * 0.5
        } else {
            ivals[mid]
        };
        if median_dt <= 1e-3 {
            return None;
        }
        Some(60.0 / median_dt)
    }
}

/// Sub-frame interpolation factor in `[0, 1]` between FFT frames.
pub fn sub_frame_t(now: Instant, frame_ts: Instant, period: Duration) -> f32 {
    if period.is_zero() {
        return 1.0;
    }
    let num = now.saturating_duration_since(frame_ts).as_secs_f32();
    let den = period.as_secs_f32().max(1e-9);
    (num / den).clamp(0.0, 1.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn asymmetric_ema_attack_faster_than_release() {
        let up = asymmetric_ema(0.0, 1.0, 0.5, 0.15);
        assert!(up > 0.4, "attack step: {up}");
        let down = asymmetric_ema(1.0, 0.0, 0.5, 0.15);
        assert!(down > 0.8, "release should be slow: {down}");
    }

    #[test]
    fn peak_hold_drift_decays() {
        assert!((peak_hold_drift(1.0, 0.0, 0.96) - 0.96).abs() < 1e-5);
    }

    #[test]
    fn one_euro_reduces_step_jitter() {
        let mut f = OneEuroFilter::new(30.0, 1.0, 0.007);
        let mut t = Instant::now();
        let mut raw = Vec::new();
        let mut filt = Vec::new();
        for i in 0..100 {
            t += Duration::from_millis(16);
            let x = if i % 2 == 0 { 0.0 } else { 1.0 };
            raw.push(x);
            filt.push(f.filter(x, t));
        }
        let var_raw: f32 = variance(&raw);
        let var_f: f32 = variance(&filt);
        assert!(var_f < var_raw, "raw var {var_raw} filt var {var_f}");
    }

    fn variance(xs: &[f32]) -> f32 {
        let m = xs.iter().sum::<f32>() / xs.len() as f32;
        xs.iter().map(|x| (x - m).powi(2)).sum::<f32>() / xs.len() as f32
    }

    #[test]
    fn beat_detector_fires_on_impulse() {
        let mut d = SpectralFluxBeatDetector::new(43);
        let low = vec![0.01f32; 32];
        let mut t = Instant::now();
        for _ in 0..50 {
            t += Duration::from_millis(33);
            let _ = d.ingest(&low, t);
        }
        t += Duration::from_millis(33);
        let impulse = vec![1.0f32; 32];
        let (beat, _) = d.ingest(&impulse, t);
        assert!(beat, "expected beat on impulse");
    }

    #[test]
    fn beat_detector_respects_250ms_min_interval() {
        let mut d = SpectralFluxBeatDetector::new(43);
        let low = vec![0.01f32; 32];
        let impulse = vec![1.0f32; 32];
        let mut t = Instant::now();
        for _ in 0..50 {
            t += Duration::from_millis(10);
            let _ = d.ingest(&low, t);
        }
        t += Duration::from_millis(10);
        let (b1, _) = d.ingest(&impulse, t);
        assert!(b1);
        t += Duration::from_millis(100);
        let (b2, _) = d.ingest(&impulse, t);
        assert!(!b2, "second beat within 250ms should be suppressed");
    }

    #[test]
    fn beat_detector_double_impulse_within_window_only_fires_once() {
        let mut d = SpectralFluxBeatDetector::new(43);
        let low = vec![0.01f32; 32];
        let impulse = vec![1.0f32; 32];
        let mut t = Instant::now();
        for _ in 0..50 {
            t += Duration::from_millis(10);
            let _ = d.ingest(&low, t);
        }
        t += Duration::from_millis(10);
        let (b1, _) = d.ingest(&impulse, t);
        assert!(b1);
        t += Duration::from_millis(50);
        let (b2, _) = d.ingest(&impulse, t);
        assert!(!b2, "still within 250ms window");
        // Let flux history decay so the next impulse crosses threshold again.
        t += Duration::from_millis(300);
        for _ in 0..80 {
            t += Duration::from_millis(10);
            let _ = d.ingest(&low, t);
        }
        t += Duration::from_millis(10);
        let (b3, _) = d.ingest(&impulse, t);
        assert!(b3, "after interval + quiet frames a new beat should be allowed");
    }

    #[test]
    fn sub_frame_t_clamps() {
        let ts = Instant::now();
        assert!((sub_frame_t(ts, ts, Duration::from_millis(33)) - 0.0).abs() < 1e-4);
        let later = ts + Duration::from_secs(10);
        assert!((sub_frame_t(later, ts, Duration::from_millis(33)) - 1.0).abs() < 1e-4);
        assert!((sub_frame_t(ts, ts, Duration::ZERO) - 1.0).abs() < 1e-4);
    }

    #[test]
    fn treble_tilt_lifts_high_bins_more_than_low_bins() {
        let mut v = vec![0.5f32; 8];
        treble_tilt(&mut v, 0.85);
        assert!(v[7] > v[0], "last bin should exceed first: {:?} ", v);
        assert!(v[0] >= 0.5 && v[0] <= 0.51);
    }

    #[test]
    fn auto_gain_amplifies_quiet_to_target() {
        let mut ag = AutoGain::new();
        let quiet = vec![0.1f32; 32];
        let mut g = 1.0f32;
        for _ in 0..120 {
            g = ag.apply(&quiet);
            if g > 2.0 {
                break;
            }
        }
        assert!(
            g > 2.0,
            "quiet peak should ramp gain toward target over frames, got {g}"
        );
    }

    #[test]
    fn auto_gain_clamps_to_max() {
        let mut ag = AutoGain::new();
        let tiny = vec![0.001f32; 8];
        for _ in 0..200 {
            ag.apply(&tiny);
        }
        assert!(ag.gain() <= 4.01, "gain should clamp to max, got {}", ag.gain());
    }

    #[test]
    fn auto_gain_attenuates_when_peak_exceeds_target() {
        let mut ag = AutoGain::new();
        for _ in 0..40 {
            ag.apply(&[1.0f32; 8]);
        }
        let g = ag.gain();
        assert!(
            g < 1.0,
            "loud peak should pull gain below 1.0, got {g}"
        );
        assert!(
            g >= 0.49,
            "gain should stay at or above min=0.5, got {g}"
        );
    }

    #[test]
    fn auto_gain_releases_faster_after_loud_transient() {
        let mut ag = AutoGain::new();
        for _ in 0..80 {
            ag.apply(&[0.05f32; 4]);
        }
        let g_high = ag.gain();
        assert!(
            g_high > 3.0,
            "quiet passage should ramp gain high, got {g_high}"
        );
        ag.apply(&[1.0f32; 4]);
        let g_after_loud = ag.gain();
        assert!(
            g_after_loud < g_high,
            "gain should drop after loud frame: {g_high} -> {g_after_loud}"
        );
    }
}
