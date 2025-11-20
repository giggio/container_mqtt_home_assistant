#[cfg(debug_assertions)]
use std::sync::atomic::AtomicUsize;
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};
use tokio::sync::{Notify, RwLock};

pub type Result<T> = std::result::Result<T, Error>;

#[derive(thiserror::Error, Debug)]
pub enum Error {
    #[error("Cancellation Requested")]
    CancellationRequested,
}

#[derive(Clone)]
pub struct CancellationTokenSource {
    tokens: Arc<RwLock<Vec<CancellationToken>>>,
    is_cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
    #[cfg(debug_assertions)]
    waiters_count: Arc<AtomicUsize>,
}

impl CancellationTokenSource {
    pub fn new() -> Self {
        Self {
            tokens: Arc::new(RwLock::new(Vec::new())),
            is_cancelled: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
            #[cfg(debug_assertions)]
            waiters_count: Arc::new(AtomicUsize::new(0)),
        }
    }

    pub async fn cancel(&mut self) {
        let mut tokens = self.tokens.write().await;
        trace!(
            "Cancelling CancellationTokenSource. Current tokens: {}, current waiters: {}",
            tokens.len(),
            {
                #[cfg(debug_assertions)]
                {
                    self.waiters_count.load(Ordering::SeqCst)
                }
                #[cfg(not(debug_assertions))]
                {
                    "unknown"
                }
            }
        );
        if self.is_cancelled.load(Ordering::SeqCst) {
            trace!("CancellationTokenSource was already cancelled, returning...");
            return;
        }
        self.is_cancelled.store(true, Ordering::SeqCst);
        for token in tokens.iter_mut() {
            trace!("Cancelling token...");
            token.cancel();
        }
        trace!("Notifying waiters...");
        self.notify.notify_waiters();
        trace!("CancellationTokenSource cancelled.");
    }

    pub async fn create_token(&mut self) -> CancellationToken {
        let token = CancellationToken {
            is_cancelled: Arc::new(AtomicBool::new(false)),
            notify: self.notify.clone(),
            #[cfg(debug_assertions)]
            waiters_count: self.waiters_count.clone(),
        };
        self.tokens.write().await.push(token.clone());
        token
    }

    #[cfg(test)]
    pub async fn token_count(&self) -> usize {
        self.tokens.read().await.len()
    }

    #[cfg(test)]
    fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::SeqCst)
    }
}

#[derive(Clone, Debug)]
pub struct CancellationToken {
    is_cancelled: Arc<AtomicBool>,
    notify: Arc<Notify>,
    #[cfg(debug_assertions)]
    waiters_count: Arc<AtomicUsize>,
}

#[cfg(test)]
impl Default for CancellationToken {
    /// The default CancellationToken will never be cancelled.
    /// To be cancellable it needs to be created from a CancellationTokenSource.
    fn default() -> Self {
        CancellationToken {
            is_cancelled: Arc::new(AtomicBool::new(false)),
            notify: Arc::new(Notify::new()),
            #[cfg(debug_assertions)]
            waiters_count: Arc::new(AtomicUsize::new(0)),
        }
    }
}

impl CancellationToken {
    fn cancel(&mut self) {
        self.is_cancelled.store(true, Ordering::SeqCst);
    }
    pub async fn wait_on<F>(&self, future: F) -> Result<<F as IntoFuture>::Output>
    where
        F: IntoFuture,
    {
        if self.is_cancelled.load(Ordering::SeqCst) {
            trace!("CancellationToken already cancelled before wait_on");
            return Err(Error::CancellationRequested);
        }
        #[cfg(debug_assertions)]
        {
            self.waiters_count.fetch_add(1, Ordering::SeqCst);
        }
        let result = tokio::select! {
            _ = self.notify.notified() => {
                trace!("CancellationToken was cancelled");
                Err(Error::CancellationRequested)
            }
            x = future.into_future() => Ok(x),
        };
        #[cfg(debug_assertions)]
        {
            self.waiters_count.fetch_sub(1, Ordering::SeqCst);
        }
        result
    }

    #[cfg(test)]
    pub fn is_cancelled(&self) -> bool {
        self.is_cancelled.load(Ordering::SeqCst)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::{Duration, sleep, timeout};

    #[tokio::test]
    async fn test_cancellation_token_source_creation() {
        let source = CancellationTokenSource::new();
        assert_eq!(source.token_count().await, 0);
        assert!(!source.is_cancelled());
    }

    #[tokio::test]
    async fn test_create_token() {
        let mut source = CancellationTokenSource::new();
        let _token1 = source.create_token().await;
        let _token2 = source.create_token().await;
        assert_eq!(source.token_count().await, 2);
    }

    #[tokio::test]
    async fn test_cancel_tokens() {
        let mut source = CancellationTokenSource::new();
        let _token = source.create_token().await;
        assert!(!source.is_cancelled());
        source.cancel().await;
        assert!(source.is_cancelled());
    }

    #[tokio::test]
    async fn test_cancel_twice() {
        let mut source = CancellationTokenSource::new();
        let _token = source.create_token().await;
        source.cancel().await;
        assert!(source.is_cancelled());
        // Cancel again should be idempotent
        source.cancel().await;
        assert!(source.is_cancelled());
    }

    #[tokio::test]
    async fn test_wait_on_completes() {
        let mut source = CancellationTokenSource::new();
        let token = source.create_token().await;

        let result = token.wait_on(async { 42 }).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_cancellation_token_source_create_several_tokens() {
        let mut source = CancellationTokenSource::new();

        // Create multiple tokens
        let _token1 = source.create_token().await;
        let _token2 = source.create_token().await;
        let _token3 = source.create_token().await;

        // All tokens should be created
        assert_eq!(source.token_count().await, 3);
        assert!(!source.is_cancelled());
    }

    #[tokio::test]
    async fn test_cancel_marks_source_as_cancelled() {
        let mut source = CancellationTokenSource::new();
        let _token = source.create_token().await;

        assert!(!source.is_cancelled());

        source.cancel().await;

        assert!(source.is_cancelled());
    }

    #[tokio::test]
    async fn test_wait_on_future_completes_before_cancel() {
        let mut source = CancellationTokenSource::new();
        let token = source.create_token().await;

        let result = token
            .wait_on(async {
                sleep(Duration::from_millis(10)).await;
                "completed"
            })
            .await;

        assert!(result.is_ok());
        assert_eq!(result.unwrap(), "completed");
    }

    #[tokio::test]
    async fn test_wait_on_different_types() {
        let mut source = CancellationTokenSource::new();
        let token = source.create_token().await;

        let int_result = token.wait_on(async { 42i32 }).await;
        assert_eq!(int_result.unwrap(), 42);

        let vec_result = token.wait_on(async { vec![1, 2, 3] }).await;
        assert_eq!(vec_result.unwrap(), vec![1, 2, 3]);
    }

    #[tokio::test]
    async fn test_multiple_tokens_independent_usage() {
        let mut source = CancellationTokenSource::new();
        let token1 = source.create_token().await;
        let token2 = source.create_token().await;

        let result1 = token1.wait_on(async { 1 }).await;
        let result2 = token2.wait_on(async { 2 }).await;

        assert_eq!(result1.unwrap(), 1);
        assert_eq!(result2.unwrap(), 2);
        assert!(!source.is_cancelled());
    }

    #[tokio::test]
    async fn test_token_clone_works() {
        let mut source = CancellationTokenSource::new();
        let token = source.create_token().await;
        let token_clone = token.clone();

        // Both tokens should work independently for successful operations
        let result1 = token.wait_on(async { "test" }).await;
        assert!(result1.is_ok());

        source.cancel().await;
        assert!(token.is_cancelled());
        assert!(token_clone.is_cancelled());
    }

    #[tokio::test]
    async fn test_wait_on_with_async_operation() {
        let mut source = CancellationTokenSource::new();
        let token = source.create_token().await;

        async fn async_operation() -> i32 {
            sleep(Duration::from_millis(50)).await;
            42
        }

        let result = token.wait_on(async_operation()).await;
        assert!(result.is_ok());
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn test_cancel_idempotence() {
        let mut source = CancellationTokenSource::new();
        let token = source.create_token().await;

        source.cancel().await;
        assert!(source.is_cancelled());
        assert!(token.is_cancelled());

        source.cancel().await;
        assert!(source.is_cancelled());
        assert!(token.is_cancelled());

        source.cancel().await;
        assert!(source.is_cancelled());
        assert!(token.is_cancelled());
    }

    #[tokio::test]
    async fn test_empty_token_source_cancel() {
        let mut source = CancellationTokenSource::new();
        assert_eq!(source.token_count().await, 0);
        source.cancel().await;
        assert!(source.is_cancelled());
    }

    #[tokio::test]
    async fn test_token_reuse() {
        let mut source = CancellationTokenSource::new();
        let token = source.create_token().await;

        let result1 = token.wait_on(async { 1 }).await;
        assert_eq!(result1.unwrap(), 1);

        let result2 = token.wait_on(async { 2 }).await;
        assert_eq!(result2.unwrap(), 2);

        let result3 = token.wait_on(async { 3 }).await;
        assert_eq!(result3.unwrap(), 3);
    }

    #[tokio::test]
    async fn test_source_with_many_tokens() {
        let mut source = CancellationTokenSource::new();
        let mut tokens = vec![];
        for _ in 0..10 {
            let token = source.create_token().await;
            tokens.push(token);
        }

        assert_eq!(source.token_count().await, 10);

        source.cancel().await;
        assert!(source.is_cancelled());
        assert!(tokens.iter().all(|t| t.is_cancelled()));
    }

    #[tokio::test]
    async fn cancel_awaited_task() {
        let mut source = CancellationTokenSource::new();
        let token = source.create_token().await;

        let new_thread_join_handle = tokio::spawn(async move {
            token
                .wait_on(async {
                    sleep(Duration::from_secs(10)).await;
                    "execution will never get here"
                })
                .await
                .map_err(|e| e.to_string())
        });
        sleep(Duration::from_millis(10)).await;
        source.cancel().await;
        let thread_result = timeout(Duration::from_millis(100), new_thread_join_handle)
            .await
            .unwrap();
        assert!(thread_result.is_ok(), "Thread panicked");
        let wait_on_result = thread_result.unwrap();
        assert!(wait_on_result.is_err(), "Async function should have been cancelled");
        assert_eq!(wait_on_result.err().unwrap(), "Cancellation Requested");
    }
}
