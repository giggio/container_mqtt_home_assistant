use std::sync::Arc;
use tokio::sync::{Notify, RwLock};

pub struct CancellationTokenSource {
    tokens: Vec<CancellationToken>,
    is_cancelled: bool,
    notify: Arc<Notify>,
}

impl CancellationTokenSource {
    pub fn new() -> Self {
        Self {
            tokens: Vec::new(),
            is_cancelled: false,
            notify: Arc::new(Notify::new()),
        }
    }
    pub async fn cancel(&mut self) {
        trace!(
            "Cancelling CancellationTokenSource. Current tokens: {}",
            self.tokens.len()
        );
        if self.is_cancelled {
            trace!("CancellationTokenSource was already cancelled, returning...");
            return;
        }
        self.is_cancelled = true;
        for token in self.tokens.iter_mut() {
            trace!("Cancelling token...");
            token.cancel().await;
        }
        trace!("Notifying waiters...");
        self.notify.notify_waiters();
        trace!("CancellationTokenSource cancelled.");
    }

    pub fn create_token(&mut self) -> CancellationToken {
        let token = CancellationToken {
            is_cancelled: Arc::new(RwLock::new(false)),
            notify: self.notify.clone(),
        };
        self.tokens.push(token.clone());
        token
    }
}

#[derive(Clone)]
pub struct CancellationToken {
    is_cancelled: Arc<RwLock<bool>>,
    notify: Arc<Notify>,
}

impl CancellationToken {
    async fn cancel(&mut self) {
        *self.is_cancelled.write().await = true;
    }
    pub async fn wait_on<F>(
        &self,
        future: F,
    ) -> Result<<F as std::future::IntoFuture>::Output, Box<dyn std::error::Error>>
    where
        F: std::future::IntoFuture,
    {
        tokio::select! {
            _ = self.notify.notified() => {
                trace!("CancellationToken was cancelled");
                Err("Cancelled".into())
            }
            x = future.into_future() => Ok(x),
        }
    }
}
