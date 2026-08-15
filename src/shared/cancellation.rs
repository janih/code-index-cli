//! Cooperative cancellation token.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use tokio::sync::Notify;

#[derive(Clone, Default)]
pub struct CancellationToken {
    inner: Arc<TokenInner>,
}

#[derive(Default)]
struct TokenInner {
    cancelled: AtomicBool,
    notify: Notify,
}

impl CancellationToken {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn cancel(&self) {
        self.inner.cancelled.store(true, Ordering::SeqCst);
        self.inner.notify.notify_waiters();
    }

    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::SeqCst)
    }

    /// Completes once the token is cancelled. Returns immediately when the
    /// token has already been cancelled.
    pub async fn cancelled(&self) {
        loop {
            if self.is_cancelled() {
                return;
            }
            self.inner.notify.notified().await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn starts_uncancelled() {
        assert!(!CancellationToken::new().is_cancelled());
    }

    #[tokio::test]
    async fn cancelled_future_completes_after_cancel() {
        let token = CancellationToken::new();
        let waiter = {
            let token = token.clone();
            tokio::spawn(async move {
                token.cancelled().await;
            })
        };
        tokio::task::yield_now().await;
        token.cancel();
        waiter.await.unwrap();
    }

    #[tokio::test]
    async fn cancelled_future_returns_immediately_when_already_cancelled() {
        let token = CancellationToken::new();
        token.cancel();
        token.cancelled().await; // must not hang
    }
}
