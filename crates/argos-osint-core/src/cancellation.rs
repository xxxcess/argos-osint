//! Shared cancellation for workers and tool calls.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

static CANCEL_NOTIFIERS: std::sync::OnceLock<
    std::sync::Mutex<
        std::collections::HashMap<usize, (std::sync::Arc<tokio::sync::Notify>, usize)>,
    >,
> = std::sync::OnceLock::new();

fn cancel_notifier_map() -> &'static std::sync::Mutex<
    std::collections::HashMap<usize, (std::sync::Arc<tokio::sync::Notify>, usize)>,
> {
    CANCEL_NOTIFIERS.get_or_init(|| std::sync::Mutex::new(std::collections::HashMap::new()))
}

/// Signals cancellation to both the atomic flag and any event-driven waiters.
pub fn cancel_with_notify(cancel: &Arc<AtomicBool>) {
    cancel.store(true, Ordering::SeqCst);
    let key = Arc::as_ptr(cancel) as usize;
    if let Ok(map) = cancel_notifier_map().lock() {
        if let Some((notify, _)) = map.get(&key) {
            notify.notify_waiters();
        }
    }
}

/// RAII guard that binds a cancellation token to the global `CANCEL_NOTIFIERS` registry.
///
/// On construction, registers the token (incrementing the reference count for its address)
/// and retrieves the shared `Arc<tokio::sync::Notify>`.
/// When dropped—whether on normal completion, early return, future drop across `tokio::select!`,
/// or unwinding—automatically unregisters the token, decrementing the reference count and
/// purging the entry when no active waiters remain.
///
/// Holds an owned `Arc<AtomicBool>` to ensure the underlying allocation address remains
/// valid and cannot be reallocated (preventing ABA key collisions) while registered.
#[must_use = "cancellation notifier guard must be held across the async wait scope to maintain registration"]
struct CancellationNotifierGuard {
    token: Arc<AtomicBool>,
    notify: Arc<tokio::sync::Notify>,
}

impl CancellationNotifierGuard {
    fn new(token: &Arc<AtomicBool>) -> Self {
        let key = Arc::as_ptr(token) as usize;
        let mut map = cancel_notifier_map().lock().expect("cancel notifier mutex");
        let entry = map
            .entry(key)
            .or_insert_with(|| (Arc::new(tokio::sync::Notify::new()), 0));
        entry.1 += 1;
        let notify = Arc::clone(&entry.0);
        Self {
            token: Arc::clone(token),
            notify,
        }
    }

    fn notified(&self) -> tokio::sync::futures::Notified<'_> {
        self.notify.notified()
    }
}

impl Drop for CancellationNotifierGuard {
    fn drop(&mut self) {
        unregister_cancellation_notifier(&self.token);
    }
}

fn unregister_cancellation_notifier(cancel: &Arc<AtomicBool>) {
    let key = Arc::as_ptr(cancel) as usize;
    if let Ok(mut map) = cancel_notifier_map().lock() {
        if let std::collections::hash_map::Entry::Occupied(mut occ) = map.entry(key) {
            occ.get_mut().1 = occ.get().1.saturating_sub(1);
            if occ.get().1 == 0 {
                occ.remove();
            }
        }
    }
}

#[cfg(test)]
fn cancel_waiter_count(cancel: &Arc<AtomicBool>) -> usize {
    cancel_notifier_map()
        .lock()
        .expect("cancel notifier mutex")
        .get(&(Arc::as_ptr(cancel) as usize))
        .map_or(0, |(_, count)| *count)
}

pub(crate) async fn wait_cancel(token: &Arc<AtomicBool>) {
    if token.load(Ordering::Relaxed) {
        return;
    }
    let guard = CancellationNotifierGuard::new(token);
    while !token.load(Ordering::Relaxed) {
        tokio::select! {
            _ = guard.notified() => {},
            _ = tokio::time::sleep(Duration::from_millis(50)) => {},
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn event_driven_cancellation_notifies_waiter_promptly() {
        let cancel = Arc::new(AtomicBool::new(false));
        let cancel_clone = cancel.clone();

        let handle = tokio::spawn(async move {
            wait_cancel(&cancel_clone).await;
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        cancel_with_notify(&cancel);

        let completed = tokio::time::timeout(Duration::from_millis(100), handle).await;
        assert!(completed.is_ok(), "wait_cancel must be notified promptly");
    }

    #[tokio::test]
    async fn guard_synchronous_drop_unregisters_immediately() {
        let cancel = Arc::new(AtomicBool::new(false));
        assert_eq!(cancel_waiter_count(&cancel), 0);
        {
            let _guard = CancellationNotifierGuard::new(&cancel);
            assert_eq!(cancel_waiter_count(&cancel), 1);
        }
        assert_eq!(
            cancel_waiter_count(&cancel),
            0,
            "Guard drop must synchronously unregister its token"
        );
    }

    #[tokio::test]
    async fn wait_cancel_select_drop_cleans_up_without_leak() {
        let cancel = Arc::new(AtomicBool::new(false));
        let unrelated = Arc::new(AtomicBool::new(false));
        let unrelated_guard = CancellationNotifierGuard::new(&unrelated);
        let mut waiter = Box::pin(wait_cancel(&cancel));
        // Prove registration happened before dropping the losing select branch.
        std::future::poll_fn(|cx| {
            assert!(std::future::Future::poll(waiter.as_mut(), cx).is_pending());
            std::task::Poll::Ready(())
        })
        .await;
        assert_eq!(cancel_waiter_count(&cancel), 1);
        tokio::select! {
            biased;
            _ = std::future::ready(()) => {},
            _ = waiter => panic!("uncancelled waiter must remain pending"),
        }
        assert_eq!(
            cancel_waiter_count(&cancel),
            0,
            "select drop must synchronously unregister the losing waiter"
        );
        assert_eq!(
            cancel_waiter_count(&unrelated),
            1,
            "cleanup must preserve unrelated waiters"
        );
        drop(unrelated_guard);
        assert_eq!(cancel_waiter_count(&unrelated), 0);
    }

    #[tokio::test]
    async fn multiple_waiters_refcount_correctly() {
        let cancel = Arc::new(AtomicBool::new(false));
        let g1 = CancellationNotifierGuard::new(&cancel);
        let g2 = CancellationNotifierGuard::new(&cancel);
        assert!(
            Arc::ptr_eq(&g1.notify, &g2.notify),
            "waiters on one token share a notifier"
        );
        assert_eq!(cancel_waiter_count(&cancel), 2);
        drop(g1);
        assert_eq!(
            cancel_waiter_count(&cancel),
            1,
            "first drop preserves the other waiter"
        );
        drop(g2);
        assert_eq!(
            cancel_waiter_count(&cancel),
            0,
            "last drop removes the token entry"
        );
    }

    #[tokio::test]
    async fn pre_cancelled_token_does_not_register() {
        let cancel = Arc::new(AtomicBool::new(true));
        wait_cancel(&cancel).await;
        assert_eq!(
            cancel_waiter_count(&cancel),
            0,
            "pre-cancelled token must not register"
        );
    }
}
