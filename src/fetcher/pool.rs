//! Bounded concurrent download queue.

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};

use crossbeam_channel::Sender;
use url::Url;

use super::downloader::{FetchEvent, FetchOpts, Fetcher};

struct PendingJob {
    url: Url,
    opts: FetchOpts,
    ui_tx: Sender<FetchEvent>,
}

struct QueueState {
    active: usize,
    pending: VecDeque<PendingJob>,
}

/// Runs up to `max_concurrent` yt-dlp jobs; excess URLs wait in a FIFO queue.
pub struct FetcherPool {
    max: usize,
    fetcher: Arc<dyn Fetcher + Send + Sync>,
    q: Mutex<QueueState>,
}

impl FetcherPool {
    pub fn new(max_concurrent: usize, fetcher: Arc<dyn Fetcher + Send + Sync>) -> Self {
        Self {
            max: max_concurrent.max(1),
            fetcher,
            q: Mutex::new(QueueState {
                active: 0,
                pending: VecDeque::new(),
            }),
        }
    }

    /// Enqueue a download; may start immediately or after earlier jobs finish.
    pub fn submit(self: &Arc<Self>, url: Url, opts: FetchOpts, ui_tx: Sender<FetchEvent>) {
        let job = PendingJob { url, opts, ui_tx };

        let mut st = self.q.lock().unwrap_or_else(|e| e.into_inner());
        if st.active < self.max {
            st.active += 1;
            drop(st);
            self.spawn_worker(job);
        } else {
            st.pending.push_back(job);
        }
    }

    fn spawn_worker(self: &Arc<Self>, job: PendingJob) {
        let pool = Arc::clone(self);
        let fetcher = Arc::clone(&self.fetcher);
        std::thread::spawn(move || {
            let _ = fetcher.fetch(&job.url, &job.opts, job.ui_tx.clone());

            let next = {
                let mut st = pool.q.lock().unwrap_or_else(|e| e.into_inner());
                st.active = st.active.saturating_sub(1);
                if st.active < pool.max {
                    st.pending.pop_front()
                } else {
                    None
                }
            };

            if let Some(j) = next {
                {
                    let mut st = pool.q.lock().unwrap_or_else(|e| e.into_inner());
                    st.active += 1;
                }
                pool.spawn_worker(j);
            }
        });
    }
}
