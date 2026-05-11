//! Intercept PCM samples for visualization: stereo ring buffer + sample counter.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crossbeam_queue::ArrayQueue;
use rodio::source::SeekError;
use rodio::Source;

/// One stereo sample pair `(L, R)`; mono sources duplicate into both channels.
pub type StereoSample = (f32, f32);

/// Lock-free ring shared with the FFT thread (Phase 6).
pub type SampleRing = Arc<ArrayQueue<StereoSample>>;

/// Wraps any `f32` [`Source`], taps stereo pairs into `ring` and counts raw samples in `samples_played`.
pub struct TapSource<S: Source<Item = f32>> {
    inner: S,
    ring: SampleRing,
    samples_played: Arc<AtomicU64>,
    channels: u16,
    pending_left: Option<f32>,
    /// After pushing `(L, R)` for `channels > 2`, drop this many following samples before the next frame.
    extra_skip: u8,
}

impl<S: Source<Item = f32>> TapSource<S> {
    pub fn new(inner: S, ring: SampleRing, samples_played: Arc<AtomicU64>) -> Self {
        let channels = inner.channels().max(1);
        Self {
            inner,
            ring,
            samples_played,
            channels,
            pending_left: None,
            extra_skip: 0,
        }
    }

    fn push_pair(&self, l: f32, r: f32) {
        let _ = self.ring.force_push((l, r));
    }

    fn bump_counter(&self) -> u64 {
        self.samples_played.fetch_add(1, Ordering::Relaxed) + 1
    }
}

impl<S: Source<Item = f32>> Iterator for TapSource<S> {
    type Item = f32;

    fn next(&mut self) -> Option<Self::Item> {
        let sample = self.inner.next()?;
        self.bump_counter();

        if self.extra_skip > 0 {
            self.extra_skip -= 1;
            return Some(sample);
        }

        match self.channels {
            1 => {
                self.push_pair(sample, sample);
            }
            _ => {
                // Stereo or surround: first two samples of each frame are L/R; rest skipped via `extra_skip`.
                if self.pending_left.is_none() {
                    self.pending_left = Some(sample);
                } else {
                    let l = self.pending_left.take().unwrap();
                    let r = sample;
                    self.push_pair(l, r);
                    if self.channels > 2 {
                        self.extra_skip = (self.channels - 2) as u8;
                    }
                }
            }
        }

        Some(sample)
    }
}

impl<S: Source<Item = f32>> Source for TapSource<S> {
    #[inline]
    fn current_frame_len(&self) -> Option<usize> {
        self.inner.current_frame_len()
    }

    #[inline]
    fn channels(&self) -> u16 {
        self.inner.channels()
    }

    #[inline]
    fn sample_rate(&self) -> u32 {
        self.inner.sample_rate()
    }

    #[inline]
    fn total_duration(&self) -> Option<Duration> {
        self.inner.total_duration()
    }

    fn try_seek(&mut self, pos: Duration) -> Result<(), SeekError> {
        self.pending_left = None;
        self.extra_skip = 0;
        self.inner.try_seek(pos)
    }
}

/// Derive playback time from raw sample count, sample rate, and channel count.
#[inline]
pub fn position_secs(samples_played: u64, sample_rate: u32, channels: u16) -> f64 {
    if sample_rate == 0 || channels == 0 {
        return 0.0;
    }
    samples_played as f64 / f64::from(sample_rate) / f64::from(channels)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixed-channel iterator for tests.
    struct TestSource {
        channels: u16,
        samples: Vec<f32>,
        idx: usize,
    }

    impl TestSource {
        fn new(channels: u16, samples: Vec<f32>) -> Self {
            Self {
                channels,
                samples,
                idx: 0,
            }
        }
    }

    impl Iterator for TestSource {
        type Item = f32;

        fn next(&mut self) -> Option<Self::Item> {
            let s = *self.samples.get(self.idx)?;
            self.idx += 1;
            Some(s)
        }
    }

    impl Source for TestSource {
        fn current_frame_len(&self) -> Option<usize> {
            None
        }

        fn channels(&self) -> u16 {
            self.channels
        }

        fn sample_rate(&self) -> u32 {
            48_000
        }

        fn total_duration(&self) -> Option<Duration> {
            None
        }
    }

    #[test]
    fn mono_duplicates_to_stereo_pair() {
        let ring = Arc::new(ArrayQueue::new(16));
        let ctr = Arc::new(AtomicU64::new(0));
        let inner = TestSource::new(1, vec![0.5, 0.5, 0.5, 0.5]);
        let mut tap = TapSource::new(inner, ring.clone(), ctr.clone());
        for _ in 0..4 {
            assert!(tap.next().is_some());
        }
        assert_eq!(ctr.load(Ordering::Relaxed), 4);
        assert_eq!(ring.len(), 4);
        for _ in 0..4 {
            let (l, r) = ring.pop().unwrap();
            assert!((l - 0.5).abs() < 1e-6 && (r - 0.5).abs() < 1e-6);
        }
    }

    #[test]
    fn stereo_pairs_consecutive_samples() {
        let ring = Arc::new(ArrayQueue::new(16));
        let ctr = Arc::new(AtomicU64::new(0));
        let inner = TestSource::new(2, vec![0.1, 0.2, 0.3, 0.4]);
        let mut tap = TapSource::new(inner, ring.clone(), ctr.clone());
        for _ in 0..4 {
            tap.next().unwrap();
        }
        assert_eq!(ring.pop(), Some((0.1, 0.2)));
        assert_eq!(ring.pop(), Some((0.3, 0.4)));
        assert_eq!(ctr.load(Ordering::Relaxed), 4);
    }

    #[test]
    fn force_push_drops_oldest_when_full() {
        let ring = Arc::new(ArrayQueue::new(2));
        let ctr = Arc::new(AtomicU64::new(0));
        let inner = TestSource::new(1, vec![1.0, 2.0, 3.0, 4.0, 5.0]);
        let mut tap = TapSource::new(inner, ring.clone(), ctr.clone());
        for _ in 0..5 {
            tap.next().unwrap();
        }
        assert_eq!(ring.len(), 2);
        assert_eq!(ring.pop(), Some((4.0, 4.0)));
        assert_eq!(ring.pop(), Some((5.0, 5.0)));
    }

    #[test]
    fn position_secs_arithmetic() {
        assert!((position_secs(48_000 * 2, 48_000, 2) - 1.0).abs() < 1e-9);
        assert_eq!(position_secs(0, 48_000, 2), 0.0);
        assert_eq!(position_secs(100, 0, 2), 0.0);
        assert_eq!(position_secs(100, 48_000, 0), 0.0);
    }
}
