//! Owned, bounded opportunistic work. These jobs are not durable user goals.
//! Rejected/cancelled jobs never count as returned; provider usage may be unknown.
use futures::FutureExt;
use std::{
    future::Future,
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::Semaphore;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

#[derive(Default)]
struct Counts {
    returned: AtomicU64,
    cancelled: AtomicU64,
    timed_out: AtomicU64,
    rejected: AtomicU64,
    panicked: AtomicU64,
}
#[derive(Debug, Clone, serde::Serialize)]
pub struct BackgroundStats {
    pub pending: usize,
    pub returned: u64,
    pub cancelled: u64,
    pub timed_out: u64,
    pub rejected: u64,
    pub panicked: u64,
}

pub struct BackgroundTasks {
    tracker: TaskTracker,
    stop: CancellationToken,
    admission: Arc<Mutex<bool>>,
    capacity: Arc<Semaphore>,
    concurrency: Arc<Semaphore>,
    counts: Arc<Counts>,
    deadline: Duration,
}
#[derive(Clone)]
pub struct BackgroundScope {
    tracker: TaskTracker,
    global_stop: CancellationToken,
    stop: CancellationToken,
    admission: Arc<Mutex<bool>>,
    capacity: Arc<Semaphore>,
    concurrency: Arc<Semaphore>,
    counts: Arc<Counts>,
    deadline: Duration,
}
impl Default for BackgroundTasks {
    fn default() -> Self {
        Self::new(4, 64, Duration::from_secs(120))
    }
}
impl BackgroundTasks {
    pub fn new(concurrency: usize, capacity: usize, deadline: Duration) -> Self {
        assert!(concurrency > 0 && capacity >= concurrency && !deadline.is_zero());
        Self {
            tracker: TaskTracker::new(),
            stop: CancellationToken::new(),
            admission: Arc::new(Mutex::new(true)),
            capacity: Arc::new(Semaphore::new(capacity)),
            concurrency: Arc::new(Semaphore::new(concurrency)),
            counts: Arc::new(Counts::default()),
            deadline,
        }
    }
    pub fn scope(&self) -> BackgroundScope {
        BackgroundScope {
            tracker: self.tracker.clone(),
            global_stop: self.stop.clone(),
            stop: self.stop.child_token(),
            admission: self.admission.clone(),
            capacity: self.capacity.clone(),
            concurrency: self.concurrency.clone(),
            counts: self.counts.clone(),
            deadline: self.deadline,
        }
    }
    pub fn stats(&self) -> BackgroundStats {
        BackgroundStats {
            pending: self.tracker.len(),
            returned: self.counts.returned.load(Ordering::Relaxed),
            cancelled: self.counts.cancelled.load(Ordering::Relaxed),
            timed_out: self.counts.timed_out.load(Ordering::Relaxed),
            rejected: self.counts.rejected.load(Ordering::Relaxed),
            panicked: self.counts.panicked.load(Ordering::Relaxed),
        }
    }
    /// Close admission, let existing jobs finish within the caller's budget,
    /// then cancel remaining futures. This pool cannot admit new jobs afterward.
    pub async fn shutdown(&self, budget: Duration) -> bool {
        {
            let mut open = self.admission.lock().unwrap_or_else(|e| e.into_inner());
            *open = false;
            self.tracker.close();
        }
        if tokio::time::timeout(budget, self.tracker.wait())
            .await
            .is_ok()
        {
            return true;
        }
        self.stop.cancel();
        // Cooperative cancellation drops awaited provider/tool futures. A CPU-bound
        // future that never yields cannot be made safe by an async timeout.
        let _ = tokio::time::timeout(Duration::from_secs(1), self.tracker.wait()).await;
        false
    }
}
impl Drop for BackgroundTasks {
    fn drop(&mut self) {
        self.stop.cancel();
        self.tracker.close();
    }
}
impl BackgroundScope {
    pub(crate) fn cancel_on_drop(&self) -> ScopeGuard {
        ScopeGuard(Some(self.stop.clone()))
    }
    pub fn cancel(&self) {
        self.stop.cancel();
    }
    pub fn spawn(
        &self,
        purpose: &'static str,
        future: impl Future<Output = ()> + Send + 'static,
    ) -> bool {
        let open = self.admission.lock().unwrap_or_else(|e| e.into_inner());
        let permit = self.capacity.clone().try_acquire_owned();
        if !*open || self.global_stop.is_cancelled() || self.stop.is_cancelled() || permit.is_err()
        {
            self.counts.rejected.fetch_add(1, Ordering::Relaxed);
            tracing::warn!(purpose, "Background work not admitted; not returned");
            return false;
        }
        let permit = permit.unwrap();
        let scope = self.clone();
        self.tracker.spawn(async move {
            let _admission_permit=permit;
            let outcome=tokio::select! {
                biased;
                _=scope.stop.cancelled() => 0,
                result=tokio::time::timeout(scope.deadline, async {
                    let _running=scope.concurrency.acquire().await.expect("background semaphore remains open");
                    std::panic::AssertUnwindSafe(future).catch_unwind().await
                }) => match result { Ok(Ok(())) => 1, Ok(Err(_)) => 3, Err(_) => 2 },
            };
            match outcome {
                1 => { scope.counts.returned.fetch_add(1, Ordering::Relaxed); }
                2 => { scope.counts.timed_out.fetch_add(1, Ordering::Relaxed); tracing::warn!(purpose,"Background deadline reached; usage/effects may be unknown"); }
                3 => { scope.counts.panicked.fetch_add(1, Ordering::Relaxed); tracing::error!(purpose,"Background work panicked; outcome not confirmed"); }
                _ => { scope.counts.cancelled.fetch_add(1, Ordering::Relaxed); tracing::info!(purpose,"Background work cancelled; usage/effects may be unknown"); }
            }
        });
        true
    }
}

/// Cancels turn-owned children if the caller drops the foreground future.
pub(crate) struct ScopeGuard(Option<CancellationToken>);
impl ScopeGuard {
    pub(crate) fn release(&mut self) {
        self.0 = None;
    }
}
impl Drop for ScopeGuard {
    fn drop(&mut self) {
        if let Some(token) = &self.0 {
            token.cancel();
        }
    }
}

/// Reset an activity flag even when an unpolled or in-flight job is dropped.
pub(crate) struct ResetFlag(pub Arc<std::sync::atomic::AtomicBool>);
impl Drop for ResetFlag {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[tokio::test]
    async fn admission_is_bounded_and_cancelled_jobs_do_not_take_effect() {
        let tasks = BackgroundTasks::new(1, 2, Duration::from_secs(10));
        let scope = tasks.scope();
        let effects = Arc::new(AtomicU64::new(0));
        for _ in 0..2 {
            let effects = effects.clone();
            assert!(scope.spawn("fixture", async move {
                tokio::time::sleep(Duration::from_secs(60)).await;
                effects.fetch_add(1, Ordering::Relaxed);
            }));
        }
        assert!(!scope.spawn("overflow", async {}));
        scope.cancel();
        assert!(tasks.shutdown(Duration::from_secs(1)).await);
        assert_eq!(effects.load(Ordering::Relaxed), 0);
        assert_eq!(tasks.stats().cancelled, 2);
        assert_eq!(tasks.stats().returned, 0);
        assert_eq!(tasks.stats().rejected, 1);
        assert!(!scope.spawn("closed", async {}));
    }
    #[tokio::test]
    async fn drain_waits_for_results_and_deadlines_are_explicit() {
        let tasks = BackgroundTasks::new(1, 2, Duration::from_millis(30));
        let scope = tasks.scope();
        assert!(scope.spawn("complete", async {}));
        assert!(scope.spawn("timeout", async {
            tokio::time::sleep(Duration::from_secs(60)).await;
        }));
        assert!(tasks.shutdown(Duration::from_secs(1)).await);
        assert_eq!(tasks.stats().returned, 1);
        assert_eq!(tasks.stats().timed_out, 1);
        assert_eq!(tasks.stats().pending, 0);
    }
    #[tokio::test]
    async fn dropping_runtime_owner_cancels_children_and_resets_flags() {
        let tasks = BackgroundTasks::default();
        let flag = Arc::new(std::sync::atomic::AtomicBool::new(true));
        let guard = ResetFlag(flag.clone());
        tasks.scope().spawn("flag", async move {
            let _guard = guard;
            std::future::pending::<()>().await;
        });
        drop(tasks);
        for _ in 0..10 {
            tokio::task::yield_now().await;
            if !flag.load(Ordering::Relaxed) {
                break;
            }
        }
        assert!(!flag.load(Ordering::Relaxed));
    }

    #[tokio::test]
    async fn panic_is_not_reported_as_a_successful_return() {
        let tasks = BackgroundTasks::default();
        assert!(tasks.scope().spawn("panic_fixture", async {
            panic!("fixture panic");
        }));
        assert!(tasks.shutdown(Duration::from_secs(1)).await);
        assert_eq!(tasks.stats().panicked, 1);
        assert_eq!(tasks.stats().returned, 0);
        assert_eq!(tasks.stats().cancelled, 0);
    }

    #[tokio::test]
    async fn dropping_one_turn_guard_preserves_another_turn() {
        let tasks = BackgroundTasks::default();
        let first = tasks.scope();
        let second = tasks.scope();
        let guard = first.cancel_on_drop();
        first.spawn("first", std::future::pending());
        second.spawn("second", async {});
        drop(guard);
        assert!(tasks.shutdown(Duration::from_secs(1)).await);
        assert_eq!(tasks.stats().cancelled, 1);
        assert_eq!(tasks.stats().returned, 1);
    }
}
