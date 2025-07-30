use std::{
    pin::Pin,
    task::{Context, Poll, Waker},
    time::Duration,
};

use tokio::time::Instant;

/// Async rate limit implementation
///
/// ```no_run
/// use std::time::Duration;
/// use rate_limit_wrapper::RateLimit;
///
/// #[tokio::main]
/// async fn main() {
///   // 10 requests per second
///   let rm = RateLimit::new(10, Duration::from_secs(1), reqwest::Client::new());
///
///   // Access inner client bypassing rate limit
///   let request = rm.as_ref().get("https://www.rust-lang.org").build().unwrap();
///
///   // Apply rate limit
///   let client = rm.rate_limit(1).await;
///   client.execute(request).await.unwrap();
/// }
/// ```
pub struct RateLimit<T> {
    inner: T,
    rate: std::sync::Mutex<RateLimitInner>,
}

struct RateLimitInner {
    size: usize,
    current: usize,
    window: Duration,
    sleep: Pin<Box<tokio::time::Sleep>>,
    wakers: Vec<Waker>,
}

impl<T> RateLimit<T> {
    #[must_use]
    pub fn new(size: usize, window: Duration, inner: T) -> Self {
        let until = Instant::now();

        RateLimit {
            inner,
            rate: std::sync::Mutex::new(RateLimitInner {
                size,
                current: size,
                window,

                // The sleep won't actually be used with this duration, but
                // we create it eagerly so that we can reset it in place rather than
                // `Box::pin`ning a new `Sleep` every time we need one.
                sleep: Box::pin(tokio::time::sleep_until(until)),

                // In parallel situation (e.g. tokio::spawn) different wakers insists on the same future,
                // if we don't collect wakers only the last one will be woke up by sleep
                wakers: vec![],
            }),
        }
    }

    #[must_use]
    pub fn rate_limit(&self, weight: usize) -> RateLimitFuture<T> {
        RateLimitFuture {
            weight,
            inner: self,
        }
    }
}

impl<T> AsRef<T> for RateLimit<T> {
    fn as_ref(&self) -> &T {
        &self.inner
    }
}

pub struct RateLimitFuture<'a, T> {
    weight: usize,
    inner: &'a RateLimit<T>,
}

impl<'a, T> Future for RateLimitFuture<'a, T> {
    type Output = &'a T;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
        let mut this = self.inner.rate.lock().expect("Poisoned lock");

        if Pin::new(&mut this.sleep).poll(cx).is_ready() {
            this.current = 0;
            let next = Instant::now() + this.window;
            this.sleep.as_mut().reset(next);

            // Wake up all waiting tasks
            this.wakers.drain(..).for_each(Waker::wake);
        }

        if this.current + self.weight <= this.size {
            this.current += self.weight;

            Poll::Ready(&self.inner.inner)
        } else {
            let waker = cx.waker();
            // Avoid registering the same waker multiple times
            // Here we consider comparing the vtable (as done in `Waker::will_wake`) as a comparison
            if !this.wakers.iter().any(|w| w.will_wake(waker)) {
                this.wakers.push(waker.clone());
            }

            Poll::Pending
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use super::*;

    #[tokio::test]
    async fn rate_limit() {
        let rt = RateLimit::new(2, Duration::from_secs(1), true);
        let start = Instant::now();
        assert!(*rt.rate_limit(1).await);
        assert!(*rt.rate_limit(1).await);
        assert!(start.elapsed().as_secs_f64() < 1.0);
        assert!(*rt.rate_limit(1).await);
        assert!(*rt.rate_limit(1).await);
        assert!(start.elapsed().as_secs_f64() > 1.0);
        assert!(*rt.rate_limit(1).await);
        assert!(*rt.rate_limit(1).await);
        assert!(start.elapsed().as_secs_f64() > 2.0);
    }

    #[tokio::test]
    async fn rate_limit_join() {
        let rt = RateLimit::new(2, Duration::from_secs(1), true);
        let start = Instant::now();
        let future1 = async {
            assert!(*rt.rate_limit(1).await);
            assert!(*rt.rate_limit(1).await);
        };
        let future2 = async {
            assert!(*rt.rate_limit(1).await);
            assert!(*rt.rate_limit(1).await);
        };
        let future3 = async {
            assert!(*rt.rate_limit(1).await);
            assert!(*rt.rate_limit(1).await);
        };
        let _ = tokio::join!(future1, future2, future3);
        assert!(start.elapsed().as_secs_f64() > 2.0);
    }

    #[tokio::test]
    async fn rate_limit_select() {
        let rt = RateLimit::new(2, Duration::from_secs(1), true);
        let start = Instant::now();
        let future1 = async {
            assert!(*rt.rate_limit(1).await);
            assert!(*rt.rate_limit(1).await);
        };
        let future2 = async {
            assert!(*rt.rate_limit(1).await);
            assert!(*rt.rate_limit(1).await);
        };
        let future3 = async {
            assert!(*rt.rate_limit(1).await);
            assert!(*rt.rate_limit(1).await);
        };
        tokio::select!(
            _ = future1 => {},
            _ = future2 => {},
            _ = future3 => {},
        );
        assert!(start.elapsed().as_secs_f64() < 1.0);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 3)]
    async fn rate_limit_spawn() {
        let rt = Arc::new(super::RateLimit::new(2, Duration::from_secs(1), true));
        let start = Instant::now();
        let future1 = tokio::spawn({
            let rt = rt.clone();
            async move {
                assert!(*rt.rate_limit(1).await);
                assert!(*rt.rate_limit(1).await);
            }
        });
        let future2 = tokio::spawn({
            let rt = rt.clone();
            async move {
                assert!(*rt.rate_limit(1).await);
                assert!(*rt.rate_limit(1).await);
            }
        });
        let future3 = tokio::spawn({
            let rt = rt.clone();
            async move {
                assert!(*rt.rate_limit(1).await);
                assert!(*rt.rate_limit(1).await);
            }
        });
        let _ = tokio::join!(future1, future2, future3);
        assert!(start.elapsed().as_secs_f64() > 2.0);
    }
}
