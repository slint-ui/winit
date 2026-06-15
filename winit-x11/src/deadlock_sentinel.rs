use std::sync::{Arc, RwLock};
use std::thread::ThreadId;

// When `thread_id_value` is stabilized, this can become `AtomicU64`.
#[derive(Default, Debug)]
pub struct DeadlockSentinel(Arc<RwLock<Option<ThreadId>>>);

/// Read-side of `DeadlockSentinel` (to prevent accidentally guarding in a re-entrant way).
#[derive(Debug, Clone)]
pub struct DeadlockSentinelReader(Arc<RwLock<Option<ThreadId>>>);

impl DeadlockSentinelReader {
    pub fn get(&self) -> Option<ThreadId> {
        *self.0.read().unwrap()
    }
}

#[must_use]
#[derive(Debug)]
pub struct DeadlockSentinelGuard(Arc<RwLock<Option<ThreadId>>>);

impl Drop for DeadlockSentinelGuard {
    fn drop(&mut self) {
        *self.0.write().unwrap() = None;
    }
}

impl DeadlockSentinel {
    pub fn guard(&self) -> DeadlockSentinelGuard {
        let mut writer = self.0.write().unwrap();
        assert!(writer.is_none(), "Internal error: re-entrant `DeadlockSentinelGuard`");
        *writer = Some(std::thread::current().id());
        DeadlockSentinelGuard(self.0.clone())
    }

    pub fn reader(&self) -> DeadlockSentinelReader {
        DeadlockSentinelReader(self.0.clone())
    }
}
