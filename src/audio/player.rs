//! Rodio-backed playback, [`AudioPlayer`] control loop, and shuffle/repeat helpers.

use std::cell::Cell;
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use crossbeam_channel::bounded;
use crossbeam_queue::ArrayQueue;
use lofty::file::AudioFile;
use lofty::probe::Probe;
use rodio::{Decoder, OutputStream, Sink, Source};

use crate::app::lock_shared;
use crate::app::state::{AppState, RepeatMode};
use crate::audio::backend::AudioBackend;
use crate::audio::tap_source::{SampleRing, TapSource};
use crate::error::{Result, RtunesError};

/// Clamp user volume to `[0.0, 1.0]`.
#[inline]
pub fn clamp_volume(v: f32) -> f32 {
    v.clamp(0.0, 1.0)
}

#[inline]
pub fn seed_xorshift() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos() as u64
        | 1
}

/// xorshift64* — deterministic PRNG for shuffle without the `rand` crate.
#[inline]
pub fn xorshift64star(state: &mut u64) -> u64 {
    *state ^= *state >> 12;
    *state ^= *state << 25;
    *state ^= *state >> 27;
    state.wrapping_mul(0x2545_F491_4F6C_DD1D)
}

/// Next index after `current` when shuffle is enabled (`len >= 2` guarantees a different index).
pub fn next_shuffled(state: &mut u64, len: usize, current: Option<usize>) -> Option<usize> {
    if len == 0 {
        return None;
    }
    if len == 1 {
        return Some(0);
    }
    let cur = current.unwrap_or(0) % len;
    let pick_offset = (xorshift64star(state) % (len as u64 - 1)) as usize + 1;
    Some((cur + pick_offset) % len)
}

fn probe_duration_secs(path: &Path) -> Option<Duration> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    let secs = tagged.properties().duration().as_secs();
    Some(Duration::from_secs(secs))
}

/// Rodio output + [`Sink`], optional silent mode when no device is available.
pub struct RodioBackend {
    _stream: Option<OutputStream>,
    sink: Option<Sink>,
    pub ring: SampleRing,
    pub samples_played: Arc<AtomicU64>,
    sample_rate: u32,
    channels: u16,
    current_path: Option<PathBuf>,
    duration: Option<Duration>,
    volume: f32,
    muted: bool,
    silent_mode: bool,
    /// Last successfully read playback position — used to resume after stream reconnection.
    last_position: Cell<Duration>,
}

impl RodioBackend {
    /// Build backend using an existing ring + counter (for the audio thread).
    pub fn new_with_ring(ring: SampleRing, samples_played: Arc<AtomicU64>) -> (Self, bool) {
        match OutputStream::try_default() {
            Ok((stream, handle)) => match Sink::try_new(&handle) {
                Ok(sink) => (
                    Self {
                        _stream: Some(stream),
                        sink: Some(sink),
                        ring,
                        samples_played,
                        sample_rate: 48_000,
                        channels: 2,
                        current_path: None,
                        duration: None,
                        volume: 1.0,
                        muted: false,
                        silent_mode: false,
                        last_position: Cell::new(Duration::ZERO),
                    },
                    false,
                ),
                Err(e) => {
                    tracing::error!(error = %e, "rodio Sink::try_new failed");
                    (
                        Self {
                            _stream: None,
                            sink: None,
                            ring,
                            samples_played,
                            sample_rate: 48_000,
                            channels: 2,
                            current_path: None,
                            duration: None,
                            volume: 1.0,
                            muted: false,
                            silent_mode: true,
                            last_position: Cell::new(Duration::ZERO),
                        },
                        true,
                    )
                }
            },
            Err(e) => {
                tracing::error!(error = %e, "rodio OutputStream::try_default failed");
                (
                    Self {
                        _stream: None,
                        sink: None,
                        ring,
                        samples_played,
                        sample_rate: 48_000,
                        channels: 2,
                        current_path: None,
                        duration: None,
                        volume: 1.0,
                        muted: false,
                        silent_mode: true,
                        last_position: Cell::new(Duration::ZERO),
                    },
                    true,
                )
            }
        }
    }

    /// Allocate ring buffer + counter, then build backend (for non-thread callers / tests on main).
    pub fn new(ring_size: usize) -> (Self, bool) {
        let ring = Arc::new(ArrayQueue::new(ring_size));
        let samples_played = Arc::new(AtomicU64::new(0));
        Self::new_with_ring(ring, samples_played)
    }

    fn resync_samples_from_logical_position(&self, logical: Duration) {
        let sr = f64::from(self.sample_rate.max(1));
        let ch = f64::from(self.channels.max(1));
        let n = (logical.as_secs_f64() * sr * ch).round() as u64;
        self.samples_played.store(n, Ordering::Relaxed);
    }

    fn build_decoder_source(
        path: &Path,
        position: Duration,
    ) -> Result<(Box<dyn Source<Item = f32> + Send>, u32, u16)> {
        let file = BufReader::new(File::open(path)?);
        let decoder = Decoder::new(file).map_err(|e| {
            RtunesError::Audio(format!(
                "Could not decode {}: {e}. Try re-encoding the file or use a supported format (mp3, flac, wav, …).",
                path.display()
            ))
        })?;
        let sample_rate = decoder.sample_rate();
        let channels = decoder.channels().max(1);
        let mut decoded = decoder.convert_samples::<f32>();
        let src: Box<dyn Source<Item = f32> + Send> = if decoded.try_seek(position).is_ok() {
            Box::new(decoded)
        } else {
            let file2 = BufReader::new(File::open(path)?);
            let decoder2 = Decoder::new(file2).map_err(|e| {
                RtunesError::Audio(format!(
                    "Could not decode {} after seek rebuild: {e}.",
                    path.display()
                ))
            })?;
            Box::new(decoder2.convert_samples::<f32>().skip_duration(position))
        };
        Ok((src, sample_rate, channels))
    }

    fn load_track_inner(&mut self, path: &Path, position: Duration) -> Result<()> {
        if self.silent_mode || self.sink.is_none() {
            return Err(RtunesError::Audio(
                "No audio output device is available. Plug in headphones or speakers, or install a virtual audio device; the TUI still runs in silent mode.".into(),
            ));
        }
        let sink = self.sink.as_ref().unwrap();
        sink.clear();
        self.samples_played.store(0, Ordering::Relaxed);

        let (src, sample_rate, channels) = Self::build_decoder_source(path, position)?;
        self.sample_rate = sample_rate;
        self.channels = channels;

        self.duration = probe_duration_secs(path).or_else(|| src.total_duration());

        let tapped = TapSource::new(src, self.ring.clone(), self.samples_played.clone());
        sink.append(tapped);
        sink.set_volume(if self.muted { 0.0 } else { self.volume });
        sink.play();
        self.current_path = Some(path.to_path_buf());
        self.resync_samples_from_logical_position(position);
        Ok(())
    }
}

impl AudioBackend for RodioBackend {
    fn current_sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn play(&mut self, path: &Path) -> Result<()> {
        self.load_track_inner(path, Duration::ZERO)
    }

    fn pause(&mut self) {
        if let Some(s) = &self.sink {
            s.pause();
        }
    }

    fn resume(&mut self) {
        if let Some(s) = &self.sink {
            s.play();
        }
    }

    fn stop(&mut self) {
        if let Some(s) = &self.sink {
            s.stop();
        }
        self.current_path = None;
        self.duration = None;
        self.samples_played.store(0, Ordering::Relaxed);
    }

    fn seek(&mut self, position: Duration) -> Result<()> {
        if self.silent_mode || self.sink.is_none() {
            return Err(RtunesError::Audio(
                "No audio output device is available. Plug in headphones or speakers, or install a virtual audio device; the TUI still runs in silent mode.".into(),
            ));
        }
        let sink = self.sink.as_ref().unwrap();
        let path = self
            .current_path
            .as_ref()
            .ok_or_else(|| RtunesError::Audio("no track loaded".into()))?
            .clone();

        match sink.try_seek(position) {
            Ok(()) => {
                self.resync_samples_from_logical_position(sink.get_pos());
                Ok(())
            }
            Err(e) if e.source_intact() => {
                self.load_track_inner(&path, position)?;
                Ok(())
            }
            Err(_) => {
                self.load_track_inner(&path, position)?;
                Ok(())
            }
        }
    }

    fn position(&self) -> Duration {
        let pos = self
            .sink
            .as_ref()
            .map(|s| {
                let p = s.get_pos();
                self.resync_samples_from_logical_position(p);
                p
            })
            .unwrap_or_default();
        // Cache non-zero positions so reconnect() can resume from here.
        if pos > Duration::ZERO {
            self.last_position.set(pos);
        }
        pos
    }

    fn duration(&self) -> Option<Duration> {
        self.duration
    }

    fn set_volume(&mut self, vol: f32) {
        self.volume = clamp_volume(vol);
        if let Some(s) = &self.sink {
            s.set_volume(if self.muted { 0.0 } else { self.volume });
        }
    }

    fn mute(&mut self, muted: bool) {
        self.muted = muted;
        if let Some(s) = &self.sink {
            s.set_volume(if muted { 0.0 } else { self.volume });
        }
    }

    fn is_finished(&self) -> bool {
        match &self.sink {
            Some(s) => s.empty() && self.current_path.is_some(),
            None => true,
        }
    }

    fn is_silent(&self) -> bool {
        self.silent_mode
    }

    fn reconnect(&mut self) -> bool {
        let path = match self.current_path.clone() {
            Some(p) => p,
            None => return false,
        };
        let resume_pos = self.last_position.get();
        tracing::warn!(
            "Audio stream lost — attempting reconnect at {:?}",
            resume_pos
        );
        match OutputStream::try_default() {
            Ok((new_stream, handle)) => match Sink::try_new(&handle) {
                Ok(new_sink) => {
                    self._stream = Some(new_stream);
                    self.sink = Some(new_sink);
                    self.silent_mode = false;
                    match self.load_track_inner(&path, resume_pos) {
                        Ok(()) => {
                            tracing::info!("Audio reconnected successfully");
                            true
                        }
                        Err(e) => {
                            tracing::error!(error = %e, "Reconnect: load_track_inner failed");
                            false
                        }
                    }
                }
                Err(e) => {
                    tracing::error!(error = %e, "Reconnect: Sink::try_new failed");
                    false
                }
            },
            Err(e) => {
                tracing::error!(error = %e, "Reconnect: OutputStream::try_default failed");
                false
            }
        }
    }
}

/// Owns a backend and syncs it with [`AppState::player`].
pub struct AudioPlayer<B: AudioBackend> {
    backend: B,
    state: Arc<Mutex<AppState>>,
    sample_rate_hz: Arc<AtomicU32>,
    rng_state: u64,
    loaded_index: Option<usize>,
    /// Tick counter used to throttle periodic silent-mode reconnect attempts (~2 s interval).
    reconnect_ticks: u32,
}

impl<B: AudioBackend> AudioPlayer<B> {
    pub fn with_backend(
        backend: B,
        state: Arc<Mutex<AppState>>,
        sample_rate_hz: Arc<AtomicU32>,
    ) -> Self {
        Self {
            backend,
            state,
            sample_rate_hz,
            rng_state: seed_xorshift(),
            loaded_index: None,
            reconnect_ticks: 0,
        }
    }

    /// Spawn the production audio thread (rodio). Returns ring + sample counter + sample rate for FFT.
    pub fn spawn(
        state: Arc<Mutex<AppState>>,
        ring_size: usize,
    ) -> (
        JoinHandle<()>,
        SampleRing,
        Arc<AtomicU64>,
        Arc<AtomicU32>,
        bool,
    ) {
        let ring = Arc::new(ArrayQueue::new(ring_size));
        let samples = Arc::new(AtomicU64::new(0));
        let sample_rate_hz = Arc::new(AtomicU32::new(0));
        let ring_th = ring.clone();
        let samples_th = samples.clone();
        let sr_th = sample_rate_hz.clone();
        let (tx, rx) = bounded(1);
        let handle = std::thread::spawn(move || {
            let (backend, silent) = RodioBackend::new_with_ring(ring_th, samples_th);
            let _ = tx.send(silent);
            let mut player = AudioPlayer::with_backend(backend, state, sr_th);
            player.run();
        });
        let silent = rx.recv_timeout(Duration::from_secs(2)).unwrap_or(true);
        (handle, ring, samples, sample_rate_hz, silent)
    }

    /// Main loop: exits when `state.quit` is true.
    pub fn run(&mut self) {
        loop {
            if self.tick_once() {
                break;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    /// One control iteration (used by tests and [`run`](Self::run)).
    pub fn tick_once(&mut self) -> bool {
        let (
            is_playing,
            volume,
            muted,
            current_index,
            shuffle,
            repeat,
            lib_len,
            track_path,
            force_reconnect,
        ) = {
            let mut g = lock_shared(&self.state);
            if g.quit {
                return true;
            }
            let idx = g.player.current_index;
            let path = idx
                .and_then(|i| g.library.get(i))
                .map(|t| t.filepath.clone());
            let force = std::mem::replace(&mut g.player.force_reconnect, false);
            (
                g.player.is_playing,
                g.player.volume,
                g.player.muted,
                idx,
                g.player.shuffle,
                g.player.repeat,
                g.library.len(),
                path,
                force,
            )
        };

        // Honour an explicit reconnect request from the TUI (e.g. Ctrl+D).
        if force_reconnect {
            self.reconnect_ticks = 0;
            self.backend.reconnect();
        }

        // Periodic auto-reconnect when in silent mode: try every ~2 s (200 × 10 ms ticks).
        if self.backend.is_silent() {
            self.reconnect_ticks = self.reconnect_ticks.saturating_add(1);
            if self.reconnect_ticks >= 200 {
                self.reconnect_ticks = 0;
                self.backend.reconnect();
            }
        } else {
            self.reconnect_ticks = 0;
        }

        // Sync silent_mode to AppState so the TUI can react (toast on transition).
        let is_now_silent = self.backend.is_silent();
        {
            let mut g = lock_shared(&self.state);
            g.player.silent_mode = is_now_silent;
        }

        // Track load / change
        if lib_len > 0 {
            if let (Some(ci), Some(path)) = (current_index, track_path) {
                if self.loaded_index != Some(ci) {
                    // If no device is available, try to reclaim one before loading.
                    if self.backend.is_silent() {
                        self.backend.reconnect();
                    }
                    if self.backend.play(&path).is_ok() {
                        self.loaded_index = Some(ci);
                        self.sample_rate_hz
                            .store(self.backend.current_sample_rate().max(1), Ordering::Relaxed);
                        let dur = self
                            .backend
                            .duration()
                            .map(|d| d.as_secs_f64())
                            .unwrap_or(0.0);
                        let mut g = lock_shared(&self.state);
                        g.player.duration_secs = dur;
                    }
                }
            }
        }

        // Seek request (consume from state) — applied after load so it affects the active track.
        let seek_applied = {
            let mut g = lock_shared(&self.state);
            g.player.seek_to.take()
        };
        if let Some(secs) = seek_applied {
            let _ = self.backend.seek(Duration::from_secs_f64(secs));
        }

        self.backend.set_volume(volume);
        self.backend.mute(muted);

        if is_playing {
            self.backend.resume();
        } else {
            self.backend.pause();
        }

        let pos_secs = self.backend.position().as_secs_f64();
        {
            let mut g = lock_shared(&self.state);
            g.player.position_secs = pos_secs;
        }

        // Auto-advance when the current sink/source has finished
        if self.backend.is_finished() && is_playing && lib_len > 0 {
            // Distinguish natural track end from stream failure (e.g. device change).
            // If we're more than 2 seconds before the expected end, it's likely a broken
            // stream rather than normal completion — attempt to reconnect first.
            let dur = self
                .backend
                .duration()
                .map(|d| d.as_secs_f64())
                .unwrap_or(0.0);
            if dur > 0.0 && pos_secs < dur - 2.0 && self.backend.reconnect() {
                // Successfully reconnected — resume playing, skip auto-advance.
                return false;
            }
            let cur = current_index.unwrap_or(0);
            match repeat {
                RepeatMode::One => {
                    let _ = self.backend.seek(Duration::ZERO);
                    let pos = self.backend.position().as_secs_f64();
                    let mut g = lock_shared(&self.state);
                    g.player.position_secs = pos;
                }
                RepeatMode::Off | RepeatMode::All => {
                    let last = cur >= lib_len.saturating_sub(1);
                    let next_idx = if shuffle {
                        next_shuffled(&mut self.rng_state, lib_len, Some(cur))
                    } else if repeat == RepeatMode::All {
                        Some((cur + 1) % lib_len)
                    } else if last {
                        None
                    } else {
                        Some(cur + 1)
                    };

                    match (repeat, next_idx) {
                        (RepeatMode::Off, None) => {
                            self.backend.stop();
                            self.loaded_index = None;
                            let mut g = lock_shared(&self.state);
                            g.player.is_playing = false;
                        }
                        (RepeatMode::All, Some(next)) if next == cur && lib_len == 1 => {
                            let _ = self.backend.seek(Duration::ZERO);
                            let pos = self.backend.position().as_secs_f64();
                            let mut g = lock_shared(&self.state);
                            g.player.position_secs = pos;
                        }
                        (_, Some(next)) => {
                            let path = {
                                let g = lock_shared(&self.state);
                                g.library.get(next).map(|t| t.filepath.clone())
                            };
                            if let Some(p) = path {
                                if self.backend.play(&p).is_ok() {
                                    self.loaded_index = Some(next);
                                    self.sample_rate_hz.store(
                                        self.backend.current_sample_rate().max(1),
                                        Ordering::Relaxed,
                                    );
                                    let dur = self
                                        .backend
                                        .duration()
                                        .map(|d| d.as_secs_f64())
                                        .unwrap_or(0.0);
                                    let mut g = lock_shared(&self.state);
                                    g.player.current_index = Some(next);
                                    g.player.duration_secs = dur;
                                    g.player.position_secs = 0.0;
                                }
                            }
                        }
                        _ => {}
                    }
                }
            }
        }

        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app::state::Track;
    use crate::audio::backend::SilentBackend;
    use crate::config::{resolve_active_theme, RtunesConfig};

    fn default_cfg() -> RtunesConfig {
        const YAML: &str = include_str!(concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/assets/default_config.yaml"
        ));
        serde_yaml::from_str(YAML).expect("default config")
    }

    fn sample_state_with_two_tracks() -> Arc<Mutex<AppState>> {
        let cfg = default_cfg();
        let theme = resolve_active_theme(&cfg.theme.active, cfg.theme.custom.as_ref());
        let mut app = AppState::new(&cfg, theme);
        app.library.push(Track {
            id: "1".into(),
            filepath: PathBuf::from("t0.mp3"),
            title: "T0".into(),
            artist: None,
            album: None,
            duration_secs: 1,
        });
        app.library.push(Track {
            id: "2".into(),
            filepath: PathBuf::from("t1.mp3"),
            title: "T1".into(),
            artist: None,
            album: None,
            duration_secs: 1,
        });
        Arc::new(Mutex::new(app))
    }

    #[test]
    fn clamp_volume_extremes() {
        assert_eq!(clamp_volume(2.0), 1.0);
        assert_eq!(clamp_volume(-1.0), 0.0);
        assert_eq!(clamp_volume(0.5), 0.5);
    }

    #[test]
    fn shuffle_next_avoids_current() {
        let mut st = 0xACE1u64;
        for _ in 0..50 {
            let n = next_shuffled(&mut st, 3, Some(1)).unwrap();
            assert_ne!(n, 1);
        }
    }

    #[test]
    fn auto_advance_repeat_off_stops_at_last() {
        let state = sample_state_with_two_tracks();
        {
            let mut g = lock_shared(&state);
            g.player.current_index = Some(1);
            g.player.repeat = RepeatMode::Off;
            g.player.is_playing = true;
            g.player.seek_to = Some(1.0);
        }
        let mut backend = SilentBackend::new();
        backend.set_duration_override(Some(Duration::from_secs(1)));
        let mut player =
            AudioPlayer::with_backend(backend, state.clone(), Arc::new(AtomicU32::new(0)));
        player.tick_once();

        let g = lock_shared(&state);
        assert!(!g.player.is_playing);
        assert!(g.player.current_index.is_none() || g.player.current_index == Some(1));
    }

    #[test]
    fn auto_advance_repeat_all_wraps() {
        let state = sample_state_with_two_tracks();
        {
            let mut g = lock_shared(&state);
            g.player.current_index = Some(1);
            g.player.repeat = RepeatMode::All;
            g.player.is_playing = true;
            g.player.seek_to = Some(1.0);
        }
        let mut backend = SilentBackend::new();
        backend.set_duration_override(Some(Duration::from_secs(1)));
        let mut player =
            AudioPlayer::with_backend(backend, state.clone(), Arc::new(AtomicU32::new(0)));
        player.tick_once();

        let g = lock_shared(&state);
        assert_eq!(g.player.current_index, Some(0));
        assert!(g.player.is_playing);
    }

    #[test]
    fn auto_advance_repeat_one_reseeks() {
        let state = sample_state_with_two_tracks();
        {
            let mut g = lock_shared(&state);
            g.player.current_index = Some(0);
            g.player.repeat = RepeatMode::One;
            g.player.is_playing = true;
            g.player.seek_to = Some(10.0);
        }
        let mut backend = SilentBackend::new();
        backend.set_duration_override(Some(Duration::from_secs(10)));
        let mut player =
            AudioPlayer::with_backend(backend, state.clone(), Arc::new(AtomicU32::new(0)));
        player.tick_once();

        let g = lock_shared(&state);
        assert_eq!(g.player.current_index, Some(0));
        assert!(g.player.is_playing);
        assert!((g.player.position_secs - 0.0).abs() < 1e-6);
    }
}
