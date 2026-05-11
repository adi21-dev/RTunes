//! Playback abstraction: real rodio backend vs headless `SilentBackend` for tests.

use std::path::{Path, PathBuf};
use std::time::Duration;

use lofty::file::AudioFile;
use lofty::probe::Probe;

use crate::error::Result;

/// Minimal playback surface for [`crate::audio::AudioPlayer`] and unit tests.
///
/// Production builds use [`crate::audio::player::RodioBackend`]; tests use [`SilentBackend`] for deterministic,
/// device-free playback state transitions.
pub trait AudioBackend {
    /// Load and start playback from `path` (or return [`crate::error::RtunesError::Audio`]).
    fn play(&mut self, path: &Path) -> Result<()>;
    /// Pause playback without unloading the current decoder.
    fn pause(&mut self);
    fn resume(&mut self);
    /// Stop and clear the current track.
    fn stop(&mut self);
    fn seek(&mut self, position: Duration) -> Result<()>;
    fn position(&self) -> Duration;
    fn duration(&self) -> Option<Duration>;
    fn set_volume(&mut self, vol: f32);
    fn mute(&mut self, muted: bool);
    fn is_finished(&self) -> bool;

    /// Attempt to reconnect after an audio device change or stream failure.
    /// Returns `true` if reconnection succeeded and playback was resumed.
    /// The default implementation is a no-op (used by `SilentBackend` and tests).
    fn reconnect(&mut self) -> bool {
        false
    }

    /// Returns `true` when no audio output device is available (silent mode).
    /// The default is `false`; `RodioBackend` reports its actual state.
    fn is_silent(&self) -> bool {
        false
    }

    /// Source sample rate for the current decoder (Hz). Return `0` if unknown / no track.
    fn current_sample_rate(&self) -> u32 {
        0
    }
}

fn probe_duration_secs(path: &Path) -> Option<Duration> {
    let tagged = Probe::open(path).ok()?.read().ok()?;
    let secs = tagged.properties().duration().as_secs();
    Some(Duration::from_secs(secs))
}

/// Headless backend: no audio device; obeys control/state transitions for unit tests.
pub struct SilentBackend {
    playing: bool,
    /// User volume when unmuted (clamped).
    volume: f32,
    muted: bool,
    position: Duration,
    duration: Option<Duration>,
    current: Option<PathBuf>,
    /// When set, `play` uses this instead of probing the file (for deterministic tests).
    duration_override: Option<Duration>,
}

impl Default for SilentBackend {
    fn default() -> Self {
        Self {
            playing: false,
            volume: 1.0,
            muted: false,
            position: Duration::ZERO,
            duration: None,
            current: None,
            duration_override: None,
        }
    }
}

impl SilentBackend {
    pub fn new() -> Self {
        Self::default()
    }

    #[cfg(test)]
    pub fn set_duration_override(&mut self, d: Option<Duration>) {
        self.duration_override = d;
    }

    #[cfg(test)]
    pub fn effective_volume(&self) -> f32 {
        if self.muted {
            0.0
        } else {
            self.volume
        }
    }

    #[cfg(test)]
    pub fn logical_volume(&self) -> f32 {
        self.volume
    }
}

impl AudioBackend for SilentBackend {
    fn play(&mut self, path: &Path) -> Result<()> {
        self.current = Some(path.to_path_buf());
        self.position = Duration::ZERO;
        self.duration = self.duration_override.or_else(|| probe_duration_secs(path));
        self.playing = true;
        Ok(())
    }

    fn pause(&mut self) {
        self.playing = false;
    }

    fn resume(&mut self) {
        self.playing = true;
    }

    fn stop(&mut self) {
        self.playing = false;
        self.current = None;
        self.position = Duration::ZERO;
        self.duration = None;
    }

    fn seek(&mut self, position: Duration) -> Result<()> {
        let max = self.duration.unwrap_or(Duration::from_secs(u64::MAX / 4));
        self.position = position.min(max);
        Ok(())
    }

    fn position(&self) -> Duration {
        self.position
    }

    fn duration(&self) -> Option<Duration> {
        self.duration
    }

    fn set_volume(&mut self, vol: f32) {
        self.volume = vol.clamp(0.0, 1.0);
    }

    fn mute(&mut self, muted: bool) {
        if muted && !self.muted {
            self.muted = true;
        } else if !muted && self.muted {
            self.muted = false;
        }
    }

    fn is_finished(&self) -> bool {
        match self.duration {
            Some(d) if d > Duration::ZERO => self.position >= d,
            // Unknown / zero-length duration: do not treat as finished (avoids tight loops).
            _ => false,
        }
    }

    fn current_sample_rate(&self) -> u32 {
        48_000
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn silent_play_pause_resume() {
        let mut b = SilentBackend::new();
        b.play(Path::new("x.mp3")).unwrap();
        assert!(b.playing);
        b.pause();
        assert!(!b.playing);
        b.resume();
        assert!(b.playing);
    }

    #[test]
    fn silent_seek_clamps() {
        let mut b = SilentBackend::new();
        b.set_duration_override(Some(Duration::from_secs(60)));
        b.play(Path::new("x.mp3")).unwrap();
        b.seek(Duration::from_secs(120)).unwrap();
        assert_eq!(b.position(), Duration::from_secs(60));
        b.seek(Duration::ZERO).unwrap();
        assert_eq!(b.position(), Duration::ZERO);
    }

    #[test]
    fn silent_volume_clamps() {
        let mut b = SilentBackend::new();
        b.set_volume(2.0);
        assert!((b.logical_volume() - 1.0).abs() < f32::EPSILON);
        b.set_volume(-0.5);
        assert!((b.logical_volume() - 0.0).abs() < f32::EPSILON);
    }

    #[test]
    fn silent_mute_preserves_prior_volume() {
        let mut b = SilentBackend::new();
        b.set_volume(0.7);
        b.mute(true);
        assert!((b.effective_volume() - 0.0).abs() < 1e-6);
        b.mute(false);
        assert!((b.effective_volume() - 0.7).abs() < 1e-6);
    }

    #[test]
    fn silent_finishes_at_duration() {
        let mut b = SilentBackend::new();
        b.set_duration_override(Some(Duration::from_secs(10)));
        b.play(Path::new("x.mp3")).unwrap();
        b.seek(Duration::from_secs(10)).unwrap();
        assert!(b.is_finished());
    }
}
