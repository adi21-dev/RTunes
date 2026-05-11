//! Dedicated FFT worker: drains PCM ring, emits [`VisualizerData`] at ~`fft_rate_hz`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::app::state::AppState;
use crate::audio::tap_source::{SampleRing, StereoSample};
use crate::config::AudioSettings;
use crate::visualizer::data::VisualizerData;
use crate::visualizer::fft::FftCore;
use crate::visualizer::smoothing::{
    apply_spectral_smoothing, asymmetric_ema, peak_hold_drift, OneEuroFilter,
    SpectralFluxBeatDetector,
};

const K_FFT_ATTACK: f32 = 22.0;
const K_FFT_RELEASE: f32 = 6.0;
const K_PEAK_DECAY: f32 = 1.2;

pub const NUM_VIS_BINS: usize = 64;

pub struct FftHandle {
    pub join: JoinHandle<()>,
    pub rx: Receiver<Arc<VisualizerData>>,
}

/// Spawn FFT thread; join when [`AppState::quit`] is true.
#[allow(unused_variables)] // `samples_played` reserved for Phase 7 sync
pub fn spawn_fft_thread(
    state: Arc<Mutex<AppState>>,
    ring: SampleRing,
    samples_played: Arc<AtomicU64>,
    sample_rate_hz: Arc<AtomicU32>,
    audio_cfg: AudioSettings,
) -> FftHandle {
    let (tx, rx) = bounded::<Arc<VisualizerData>>(2);
    let join = std::thread::spawn(move || {
        fft_loop(
            state,
            ring,
            sample_rate_hz,
            audio_cfg,
            tx,
        );
    });
    FftHandle { join, rx }
}

fn fft_loop(
    state: Arc<Mutex<AppState>>,
    ring: SampleRing,
    sample_rate_hz: Arc<AtomicU32>,
    cfg: AudioSettings,
    tx: Sender<Arc<VisualizerData>>,
) {
    let window_size = cfg.fft_window_size.max(256) as usize;
    let window_size = window_size.next_power_of_two();
    let hop_size = (cfg.fft_hop_size as usize).min(window_size).max(1);
    let fft_rate_hz = cfg.fft_rate_hz.clamp(1, 120) as u64;
    let frame_period = Duration::from_millis(1000 / fft_rate_hz);
    let spec_rows_cap = cfg.spectrogram_history_rows.max(8) as usize;
    let pcm_snapshot = cfg.pcm_snapshot_samples.max(128) as usize;

    let mut stereo_window: VecDeque<StereoSample> = VecDeque::with_capacity(window_size * 2);
    let mut mono_window = vec![0.0f32; window_size];
    let mut bins_raw = vec![0.0f32; NUM_VIS_BINS];
    let mut bins_smoothed = vec![0.0f32; NUM_VIS_BINS];
    let mut bins_peak = vec![0.0f32; NUM_VIS_BINS];
    let mut prev_smoothed = vec![0.0f32; NUM_VIS_BINS];

    let mut fft: Option<FftCore> = None;
    let mut last_sr = 0u32;

    let mut euro_bass = OneEuroFilter::new(30.0, 1.0, 0.007);
    let mut euro_mid = OneEuroFilter::new(30.0, 1.0, 0.007);
    let mut euro_high = OneEuroFilter::new(30.0, 1.0, 0.007);
    let mut euro_loud = OneEuroFilter::new(30.0, 1.0, 0.007);
    let mut beat_det = SpectralFluxBeatDetector::new(43);

    // 3-slot object pool — zero-alloc VisualizerData delivery to TUI.
    // With channel capacity 2 and pool size 3, at least one slot always has refcount 1
    // (not held by channel or TUI renderer), so Arc::get_mut always succeeds in steady state.
    let mut pool: Vec<Arc<VisualizerData>> = (0..3)
        .map(|_| {
            let mut d = VisualizerData::empty(NUM_VIS_BINS);
            // Pre-allocate PCM vectors so first-fill never triggers a heap allocation.
            d.pcm_mono = Vec::with_capacity(pcm_snapshot);
            d.pcm_stereo = Vec::with_capacity(pcm_snapshot);
            Arc::new(d)
        })
        .collect();

    // Row recycling pool: pre-allocate spec_rows_cap rows so steady-state push is free.
    let mut row_pool: VecDeque<Vec<f32>> = VecDeque::with_capacity(spec_rows_cap + 4);
    for _ in 0..spec_rows_cap {
        row_pool.push_back(vec![0.0f32; NUM_VIS_BINS]);
    }

    loop {
        let iter_start = Instant::now();
        if state.lock().unwrap().quit {
            break;
        }

        let sr = sample_rate_hz.load(Ordering::Relaxed);
        if sr == 0 {
            std::thread::sleep(Duration::from_millis(20));
            continue;
        }

        if fft.is_none() || sr != last_sr {
            fft = Some(FftCore::new(window_size, NUM_VIS_BINS, sr));
            last_sr = sr;
        }
        let fft_core = fft.as_mut().unwrap();
        fft_core.rebuild_for_sample_rate(sr);

        while stereo_window.len() < window_size {
            match ring.pop() {
                Some(p) => stereo_window.push_back(p),
                None => break,
            }
        }

        if stereo_window.len() < window_size {
            std::thread::sleep(Duration::from_millis(1000 / fft_rate_hz.max(1)));
            continue;
        }

        for (i, p) in stereo_window.iter().take(window_size).enumerate() {
            mono_window[i] = (p.0 + p.1) * 0.5;
        }

        fft_core.process(&mut mono_window, &mut bins_raw);
        apply_spectral_smoothing(&mut bins_raw);

        let dt = frame_period.as_secs_f32();
        let attack = 1.0 - (-dt * K_FFT_ATTACK).exp();
        let release = 1.0 - (-dt * K_FFT_RELEASE).exp();
        let peak_decay = (-dt * K_PEAK_DECAY).exp();

        // --- Find a free pool slot (refcount == 1 means not held by channel or TUI) ---
        // Save prev_smoothed into the pool slot BEFORE updating it.
        let slot_idx = pool
            .iter_mut()
            .position(|arc| Arc::get_mut(arc).is_some())
            .unwrap_or_else(|| {
                // Fallback: all slots busy (shouldn't happen with pool=3, channel=2).
                let mut d = VisualizerData::empty(NUM_VIS_BINS);
                d.pcm_mono = Vec::with_capacity(pcm_snapshot);
                d.pcm_stereo = Vec::with_capacity(pcm_snapshot);
                pool.push(Arc::new(d));
                pool.len() - 1
            });

        // Write bins_prev in-place from prev_smoothed BEFORE we update prev_smoothed.
        {
            let viz = Arc::get_mut(&mut pool[slot_idx]).expect("slot exclusive");
            viz.bins_prev.copy_from_slice(&prev_smoothed);
        }

        // Compute new smoothed bins using the local scratch.
        for i in 0..NUM_VIS_BINS {
            bins_smoothed[i] = asymmetric_ema(
                prev_smoothed[i],
                bins_raw[i],
                attack,
                release,
            );
            bins_peak[i] = peak_hold_drift(bins_peak[i], bins_smoothed[i], peak_decay);
        }
        prev_smoothed.copy_from_slice(&bins_smoothed);

        let now = Instant::now();
        let (b0, m0, h0) = FftCore::band_energies(&bins_smoothed, &fft_core.log_bin_edges);
        let bass_e = euro_bass.filter(b0, now);
        let mid_e = euro_mid.filter(m0, now);
        let high_e = euro_high.filter(h0, now);
        let loud_raw = FftCore::loudness(&mono_window);
        let loud_e = euro_loud.filter(loud_raw, now);

        let (beat, beat_intensity) = beat_det.ingest(&bins_smoothed, now);
        let bpm = beat_det.bpm_estimate();

        // --- Fill remaining pool slot fields in-place (zero allocation in steady state) ---
        {
            let viz = Arc::get_mut(&mut pool[slot_idx]).expect("slot exclusive");

            viz.bins_raw.copy_from_slice(&bins_raw);
            viz.bins_smoothed.copy_from_slice(&bins_smoothed);
            viz.bins_peak.copy_from_slice(&bins_peak);
            // bins_prev already filled above.

            // PCM snapshot: reuse existing Vec capacity.
            let take = pcm_snapshot.min(stereo_window.len());
            let start_idx = stereo_window.len().saturating_sub(take);
            viz.pcm_mono.clear();
            viz.pcm_stereo.clear();
            for p in stereo_window.range(start_idx..) {
                viz.pcm_stereo.push(*p);
                viz.pcm_mono.push((p.0 + p.1) * 0.5);
            }

            // Spectrogram waterfall: recycle rows from the back into row_pool.
            // Arc::get_mut succeeds because only this VisualizerData holds spectrogram_rows
            // (the outer Arc is exclusively ours right now).
            {
                let rows = Arc::get_mut(&mut viz.spectrogram_rows)
                    .expect("spectrogram_rows exclusive");
                while rows.len() >= spec_rows_cap {
                    if let Some(old) = rows.pop_back() {
                        row_pool.push_back(old);
                    }
                }
                let mut new_row = row_pool
                    .pop_front()
                    .unwrap_or_else(|| vec![0.0f32; NUM_VIS_BINS]);
                let n = bins_smoothed.len().min(new_row.len());
                new_row[..n].copy_from_slice(&bins_smoothed[..n]);
                rows.push_front(new_row);
            }

            viz.bass_energy = bass_e;
            viz.mid_energy = mid_e;
            viz.high_energy = high_e;
            viz.loudness = loud_e;
            viz.beat = beat;
            viz.beat_intensity = beat_intensity;
            viz.bpm_estimate = bpm;
            viz.timestamp = now;
            viz.fft_period = frame_period;
        }

        // Send a clone of the Arc (cheap pointer bump, no data copy).
        let _ = tx.try_send(Arc::clone(&pool[slot_idx]));

        for _ in 0..hop_size {
            stereo_window.pop_front();
        }

        let elapsed = iter_start.elapsed();
        if elapsed < frame_period {
            std::thread::sleep(frame_period - elapsed);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{resolve_active_theme, RtunesConfig};
    use crossbeam_queue::ArrayQueue;

    fn default_cfg() -> RtunesConfig {
        const YAML: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/default_config.yaml"
        ));
        serde_yaml::from_str(YAML).expect("default config")
    }

    #[test]
    fn spawn_emits_frame_when_ring_filled() {
        let cfg = default_cfg();
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let state = Arc::new(Mutex::new(AppState::new(&cfg, theme)));
        let ring: SampleRing = Arc::new(ArrayQueue::new(16_384));
        let samples = Arc::new(AtomicU64::new(0));
        let sr = Arc::new(AtomicU32::new(48_000));

        let n = cfg.audio.fft_window_size as usize;
        let sr_f = 48_000.0f32;
        for i in 0..n {
            let t = i as f32 / sr_f;
            let s = (std::f32::consts::TAU * 1000.0 * t).sin();
            let _ = ring.force_push((s, s));
        }

        let fft = spawn_fft_thread(state.clone(), ring, samples, sr, cfg.audio);
        let frame = fft
            .rx
            .recv_timeout(Duration::from_secs(2))
            .expect("expected VisualizerData");
        assert_eq!(frame.bins_smoothed.len(), NUM_VIS_BINS);
        assert!(
            frame.bins_smoothed.iter().any(|&x| x > 1e-6),
            "expected non-flat spectrum"
        );

        state.lock().unwrap().quit = true;
        fft.join.join().expect("fft thread join");
    }
}
