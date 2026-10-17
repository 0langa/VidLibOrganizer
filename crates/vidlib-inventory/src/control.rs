use crate::ids::EntryId;
use serde::{Deserialize, Serialize};
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc::{sync_channel, Receiver, SyncSender},
    Arc, Mutex,
};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProgressPhase {
    Scanning,
    Complete,
    Cancelled,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Progress {
    pub phase: ProgressPhase,
    #[serde(with = "crate::exact::u64_decimal")]
    pub done: u64,
    #[serde(with = "crate::exact::option_u64_decimal")]
    pub total: Option<u64>,
    pub current_entry: Option<EntryId>,
}
/// Cloneable job lifetime; nonblocking, bounded progress delivery at most 10/s.
#[derive(Clone, Default)]
pub struct JobControl {
    cancellation: Arc<AtomicBool>,
    progress: Option<SyncSender<Progress>>,
    last_progress: Arc<Mutex<Option<Instant>>>,
}
impl JobControl {
    pub fn with_progress(capacity: usize) -> (Self, Receiver<Progress>) {
        let (sender, receiver) = sync_channel(capacity);
        (
            Self {
                progress: Some(sender),
                ..Self::default()
            },
            receiver,
        )
    }
    pub fn cancellation_flag(&self) -> Arc<AtomicBool> {
        Arc::clone(&self.cancellation)
    }
    pub fn cancel(&self) {
        self.cancellation.store(true, Ordering::Release);
    }
    pub fn is_cancelled(&self) -> bool {
        self.cancellation.load(Ordering::Acquire)
    }
    pub fn report_progress(&self, progress: Progress) {
        let Some(sender) = &self.progress else {
            return;
        };
        let mut last = self
            .last_progress
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        let now = Instant::now();
        if last.is_some_and(|previous| now.duration_since(previous) < Duration::from_millis(100)) {
            return;
        }
        // A full/disconnected channel drops this update without blocking traversal.
        // Count attempts too, so a slow receiver cannot cause an unbounded retry rate.
        *last = Some(now);
        let _ = sender.try_send(progress);
    }
}
