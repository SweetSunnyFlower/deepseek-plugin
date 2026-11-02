use crate::settings::AppSettings;
use regex::Regex;
use std::future::Future;
use std::sync::{Arc, Mutex as StdMutex, RwLock};
use std::time::Duration;
use tokio::sync::Mutex;
use tokio::time::Instant;

pub struct ProviderController {
    settings: Arc<RwLock<AppSettings>>,
    provider: String,
    queue: Mutex<()>,
    last_send: StdMutex<Option<Instant>>,
    backoff: [Duration; 3],
    rate_limit: Regex,
}

impl ProviderController {
    pub fn new(settings: Arc<RwLock<AppSettings>>, provider: &str) -> Self {
        Self {
            settings,
            provider: provider.to_string(),
            queue: Mutex::new(()),
            last_send: StdMutex::new(None),
            backoff: [
                Duration::from_secs(15),
                Duration::from_secs(30),
                Duration::from_secs(60),
            ],
            rate_limit: Regex::new(r"频繁|稍后重试|rate.?limit|HTTP 429").unwrap(),
        }
    }

    pub async fn enqueue<F, Fut, T>(&self, f: F) -> Result<T, String>
    where
        F: Fn() -> Fut,
        Fut: Future<Output = Result<T, String>>,
    {
        let _guard = self.queue.lock().await;
        let mut attempt = 0usize;
        loop {
            let interval = {
                let s = self.settings.read().unwrap();
                Duration::from_millis(
                    s.providers
                        .get(&self.provider)
                        .map(|p| p.min_interval_ms)
                        .unwrap_or(5000),
                )
            };
            let wait = {
                let last = *self.last_send.lock().unwrap();
                last.map(|t| interval.saturating_sub(t.elapsed()))
                    .unwrap_or_default()
            };
            if !wait.is_zero() {
                tokio::time::sleep(wait).await;
            }
            *self.last_send.lock().unwrap() = Some(Instant::now());
            match f().await {
                Ok(v) => return Ok(v),
                Err(e) if self.rate_limit.is_match(&e) && attempt < self.backoff.len() => {
                    tokio::time::sleep(self.backoff[attempt]).await;
                    attempt += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::ProviderSettings;
    use std::sync::atomic::{AtomicUsize, Ordering};

    fn test_settings(interval: u64) -> AppSettings {
        let mut s = AppSettings::default();
        s.providers.insert(
            "deepseek".into(),
            ProviderSettings {
                enabled: true,
                min_interval_ms: interval,
            },
        );
        s
    }

    #[tokio::test(start_paused = true)]
    async fn enforces_min_interval() {
        let settings = Arc::new(RwLock::new(test_settings(1000)));
        let c = ProviderController::new(settings, "deepseek");
        let start = Instant::now();
        c.enqueue(|| async { Ok::<_, String>(1) }).await.unwrap();
        let after_first = Instant::now();
        c.enqueue(|| async { Ok::<_, String>(2) }).await.unwrap();
        let after_second = Instant::now();
        assert!(after_first.duration_since(start) < Duration::from_millis(10));
        assert!(after_second.duration_since(after_first) >= Duration::from_millis(1000));
    }

    #[tokio::test(start_paused = true)]
    async fn retries_rate_limit_errors() {
        let settings = Arc::new(RwLock::new(test_settings(1000)));
        let c = ProviderController::new(settings, "deepseek");
        let attempts = Arc::new(AtomicUsize::new(0));
        let a = attempts.clone();
        let start = Instant::now();
        let out = c
            .enqueue(move || {
                let a = a.clone();
                async move {
                    let n = a.fetch_add(1, Ordering::SeqCst) + 1;
                    if n < 3 {
                        Err("消息发送过于频繁".to_string())
                    } else {
                        Ok(n)
                    }
                }
            })
            .await
            .unwrap();
        assert_eq!(out, 3);
        assert_eq!(attempts.load(Ordering::SeqCst), 3);
        assert!(start.elapsed() >= Duration::from_secs(45));
    }

    #[tokio::test(start_paused = true)]
    async fn non_rate_limit_error_returns_immediately() {
        let settings = Arc::new(RwLock::new(test_settings(1000)));
        let c = ProviderController::new(settings, "deepseek");
        let err = c
            .enqueue(|| async { Err::<(), _>("插件未连接".to_string()) })
            .await
            .unwrap_err();
        assert_eq!(err, "插件未连接");
    }
}
