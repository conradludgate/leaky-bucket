//! [<img alt="github" src="https://img.shields.io/badge/github-udoprog/leaky--bucket-8da0cb?style=for-the-badge&logo=github" height="20">](https://github.com/udoprog/leaky-bucket)
//! [<img alt="crates.io" src="https://img.shields.io/crates/v/leaky-bucket.svg?style=for-the-badge&color=fc8d62&logo=rust" height="20">](https://crates.io/crates/leaky-bucket)
//! [<img alt="docs.rs" src="https://img.shields.io/badge/docs.rs-leaky--bucket-66c2a5?style=for-the-badge&logoColor=white&logo=data:image/svg+xml;base64,PHN2ZyByb2xlPSJpbWciIHhtbG5zPSJodHRwOi8vd3d3LnczLm9yZy8yMDAwL3N2ZyIgdmlld0JveD0iMCAwIDUxMiA1MTIiPjxwYXRoIGZpbGw9IiNmNWY1ZjUiIGQ9Ik00ODguNiAyNTAuMkwzOTIgMjE0VjEwNS41YzAtMTUtOS4zLTI4LjQtMjMuNC0zMy43bC0xMDAtMzcuNWMtOC4xLTMuMS0xNy4xLTMuMS0yNS4zIDBsLTEwMCAzNy41Yy0xNC4xIDUuMy0yMy40IDE4LjctMjMuNCAzMy43VjIxNGwtOTYuNiAzNi4yQzkuMyAyNTUuNSAwIDI2OC45IDAgMjgzLjlWMzk0YzAgMTMuNiA3LjcgMjYuMSAxOS45IDMyLjJsMTAwIDUwYzEwLjEgNS4xIDIyLjEgNS4xIDMyLjIgMGwxMDMuOS01MiAxMDMuOSA1MmMxMC4xIDUuMSAyMi4xIDUuMSAzMi4yIDBsMTAwLTUwYzEyLjItNi4xIDE5LjktMTguNiAxOS45LTMyLjJWMjgzLjljMC0xNS05LjMtMjguNC0yMy40LTMzLjd6TTM1OCAyMTQuOGwtODUgMzEuOXYtNjguMmw4NS0zN3Y3My4zek0xNTQgMTA0LjFsMTAyLTM4LjIgMTAyIDM4LjJ2LjZsLTEwMiA0MS40LTEwMi00MS40di0uNnptODQgMjkxLjFsLTg1IDQyLjV2LTc5LjFsODUtMzguOHY3NS40em0wLTExMmwtMTAyIDQxLjQtMTAyLTQxLjR2LS42bDEwMi0zOC4yIDEwMiAzOC4ydi42em0yNDAgMTEybC04NSA0Mi41di03OS4xbDg1LTM4Ljh2NzUuNHptMC0xMTJsLTEwMiA0MS40LTEwMi00MS40di0uNmwxMDItMzguMiAxMDIgMzguMnYuNnoiPjwvcGF0aD48L3N2Zz4K" height="20">](https://docs.rs/leaky-bucket)
//!
//! A token-based rate limiter based on the [leaky bucket] algorithm.
//!
//! If the bucket overflows and goes over its max configured capacity, the task
//! that tried to acquire the tokens will be suspended until the required number
//! of tokens has been drained from the bucket.
//!
//! Since this crate uses timing facilities from tokio it has to be used within
//! a Tokio runtime with the [`time` feature] enabled.
//!
//! This library has some neat features, which includes:
//!
//! **Not requiring a background task**. This is usually needed by token bucket
//! rate limiters to drive progress. Instead, one of the waiting tasks
//! temporarily assumes the role as coordinator (called the *core*). This
//! reduces the amount of tasks needing to sleep, which can be a source of
//! jitter for imprecise sleeping implementations and tight limiters. See below
//! for more details.
//!
//! **Dropped tasks** release any resources they've reserved. So that
//! constructing and cancellaing asynchronous tasks to not end up taking up wait
//! slots it never uses which would be the case for cell-based rate limiters.
//!
//! <br>
//!
//! ## Usage
//!
//! The core type is [`RateLimiter`], which allows for limiting the throughput
//! of a section using its [`acquire`], [`try_acquire`], and [`acquire_one`]
//! methods.
//!
//! The following is a simple example where we wrap requests through a HTTP
//! `Client`, to ensure that we don't exceed a given limit:
//!
//! ```
//! use leaky_bucket::RateLimiter;
//! # struct Client;
//! # impl Client { async fn request<T>(&self, path: &str) -> Result<T> { todo!() } }
//! # trait DeserializeOwned {}
//! # impl DeserializeOwned for Vec<Post> {}
//! # type Result<T> = core::result::Result<T, ()>;
//!
//! /// A blog client.
//! pub struct BlogClient {
//!     limiter: RateLimiter,
//!     client: Client,
//! }
//!
//! struct Post {
//!     // ..
//! }
//!
//! impl BlogClient {
//!     /// Get all posts from the service.
//!     pub async fn get_posts(&self) -> Result<Vec<Post>> {
//!         self.request("posts").await
//!     }
//!
//!     /// Perform a request against the service, limiting requests to abide by a rate limit.
//!     async fn request<T>(&self, path: &str) -> Result<T>
//!     where
//!         T: DeserializeOwned
//!     {
//!         // Before we start sending a request, we block on acquiring one token.
//!         self.limiter.acquire(1).await;
//!         self.client.request::<T>(path).await
//!     }
//! }
//! ```
//!
//! <br>
//!
//! ## Implementation details
//!
//! Each rate limiter has two acquisition modes. A fast path and a slow path.
//! The fast path is used if the desired number of tokens are readily available,
//! and simply involves decrementing the number of tokens available in the
//! shared pool.
//!
//! If the required number of tokens is not available, the task will be forced
//! to be suspended until the next refill interval. Here one of the acquiring
//! tasks will switch over to work as a *core*. This is known as *core
//! switching*.
//!
//! ```
//! use leaky_bucket::RateLimiter;
//! use tokio::time::Duration;
//!
//! # #[tokio::main(flavor="current_thread", start_paused=true)] async fn main() {
//! let limiter = RateLimiter::builder()
//!     .initial(10)
//!     .interval(Duration::from_millis(100))
//!     .build();
//!
//! // This is instantaneous since the rate limiter starts with 10 tokens to
//! // spare.
//! limiter.acquire(10).await;
//!
//! // This however needs to core switch and wait for a while until the desired
//! // number of tokens is available.
//! limiter.acquire(3).await;
//! # }
//! ```
//!
//! The core is responsible for sleeping for the configured interval so that
//! more tokens can be added. After which it ensures that any tasks that are
//! waiting to acquire including itself are appropriately unsuspended.
//!
//! On-demand core switching is what allows this rate limiter implementation to
//! work without a coordinating background thread. But we need to ensure that
//! any asynchronous tasks that uses [`RateLimiter`] must either run an
//! [`acquire`] call to completion, or be *cancelled* by being dropped.
//!
//! If none of these hold, the core might leak and be locked indefinitely
//! preventing any future use of the rate limiter from making progress. This is
//! similar to if you would lock an asynchronous [`Mutex`] but never drop its
//! guard.
//!
//! > You can run this example with:
//! >
//! > ```sh
//! > cargo run --example block_forever
//! > ```
//!
//! ```no_run
//! use std::future::Future;
//! use std::sync::Arc;
//! use std::task::Context;
//!
//! use leaky_bucket::RateLimiter;
//!
//! struct Waker;
//! # impl std::task::Wake for Waker { fn wake(self: Arc<Self>) { } }
//!
//! # #[tokio::main(flavor="current_thread", start_paused=true)] async fn main() {
//! let limiter = Arc::new(RateLimiter::builder().build());
//!
//! let waker = Arc::new(Waker).into();
//! let mut cx = Context::from_waker(&waker);
//!
//! let mut a0 = Box::pin(limiter.acquire(1));
//! // Poll once to ensure that the core task is assigned.
//! assert!(a0.as_mut().poll(&mut cx).is_pending());
//! assert!(a0.is_core());
//!
//! // We leak the core task, preventing the rate limiter from making progress
//! // by assigning new core tasks.
//! std::mem::forget(a0);
//!
//! // Awaiting acquire here would block forever.
//! // limiter.acquire(1).await;
//! # }
//! ```
//!
//! <br>
//!
//! ## Fairness
//!
//! By default [`RateLimiter`] uses a *fair* scheduler. This ensures that the
//! core task makes progress even if there are many tasks waiting to acquire
//! tokens. This might cause more core switching, increasing the total work
//! needed. An unfair scheduler is expected to do a bit less work under
//! contention. But without fair scheduling some tasks might end up taking
//! longer to acquire than expected.
//!
//! Unfair rate limiters also have access to a fast path for acquiring tokens,
//! which might further improve throughput.
//!
//! This behavior can be tweaked with the [`Builder::fair`] option.
//!
//! ```
//! use leaky_bucket::RateLimiter;
//!
//! let limiter = RateLimiter::builder()
//!     .fair(false)
//!     .build();
//! ```
//!
//! The `unfair-scheduling` example can showcase this phenomenon.
//!
//! ```sh
//! cargo run --example unfair_scheduling
//! ```
//!
//! ```text
//! # fair
//! Max: 1011ms, Total: 1012ms
//! Timings:
//!  0: 101ms
//!  1: 101ms
//!  2: 101ms
//!  3: 101ms
//!  4: 101ms
//!  ...
//! # unfair
//! Max: 1014ms, Total: 1014ms
//! Timings:
//!  0: 1014ms
//!  1: 101ms
//!  2: 101ms
//!  3: 101ms
//!  4: 101ms
//!  ...
//! ```
//!
//! As can be seen above the first task in the *unfair* scheduler takes longer
//! to run because it prioritises releasing other tasks waiting to acquire over
//! itself.
//!
//! [`acquire_one`]: https://docs.rs/leaky-bucket/1/leaky_bucket/struct.RateLimiter.html#method.acquire_one
//! [`acquire`]: https://docs.rs/leaky-bucket/1/leaky_bucket/struct.RateLimiter.html#method.acquire
//! [`Builder::fair`]: https://docs.rs/leaky-bucket/1/leaky_bucket/struct.Builder.html#method.fair
//! [`Mutex`]: https://docs.rs/tokio/1/tokio/sync/struct.Mutex.html
//! [`RateLimiter`]: https://docs.rs/leaky-bucket/1/leaky_bucket/struct.RateLimiter.html
//! [`time` feature]: https://docs.rs/tokio/1/tokio/#feature-flags
//! [`try_acquire`]: https://docs.rs/leaky-bucket/1/leaky_bucket/struct.RateLimiter.html#method.try_acquire
//! [leaky bucket]: https://en.wikipedia.org/wiki/Leaky_bucket

#![no_std]
#![deny(missing_docs)]

extern crate alloc;

#[macro_use]
extern crate std;

use core::task::Poll;
use core::time::Duration;
use parking_lot::Mutex;
use pin_project_lite::pin_project;
use std::future::Future;
use tokio::sync::futures::Notified;
use tokio::sync::Notify;
use tokio::time::{self, Sleep};

use tokio::time::Instant;

/// Default factor for how to calculate max refill value.
const DEFAULT_REFILL_MAX_FACTOR: usize = 10;

// /// Interval to bump the shared mutex guard to allow other parts of the system
// /// to make process. Processes which loop should use this number to determine
// /// how many times it should loop before calling [Guard::bump].
// ///
// /// If we do not respect this limit we might inadvertently end up starving other
// /// tasks from making progress so that they can unblock.
// const BUMP_LIMIT: usize = 16;

/// The maximum supported balance.
const MAX_BALANCE: usize = isize::MAX as usize;

struct LeakyBucketConfig {
    /// Leaky buckets can drain at a fixed interval rate.
    /// We track all times as durations since this epoch so we can round down.
    pub epoch: Instant,

    /// How frequently we drain the bucket.
    /// If equal to 0, we drain continuously over time.
    /// If greater than 0, we drain at fixed intervals.
    pub drain_interval: Duration,

    /// "time cost" of a single request unit.
    /// should loosely represents how long it takes to handle a request unit in active resource time.
    pub cost: Duration,

    /// total size of the bucket
    pub bucket_width: Duration,
}

impl LeakyBucketConfig {
    fn prev_multiple_of_drain(&self, mut dur: Duration) -> Duration {
        if self.drain_interval > Duration::ZERO {
            let n = dur.div_duration_f64(self.drain_interval).floor();
            dur = self.drain_interval.mul_f64(n);
        }
        dur
    }

    fn next_multiple_of_drain(&self, mut dur: Duration) -> Duration {
        if self.drain_interval > Duration::ZERO {
            let n = dur.div_duration_f64(self.drain_interval).ceil();
            dur = self.drain_interval.mul_f64(n);
        }
        dur
    }
}

struct LeakyBucketState {
    /// Bucket is represented by `start..end` where `end = epoch + end` and `start = end - config.bucket_width`.
    ///
    /// At any given time, `end - now` represents the number of tokens in the bucket, multiplied by the "time_cost".
    /// Adding `n` tokens to the bucket is done by moving `end` forward by `n * config.time_cost`.
    /// If `now < start`, the bucket is considered filled and cannot accept any more tokens.
    /// Draining the bucket will happen naturally as `now` moves forward.
    ///
    /// Let `n` be some "time cost" for the request,
    /// If now is after end, the bucket is empty and the end is reset to now,
    /// If now is within the `bucket window + n`, we are within time budget.
    /// If now is before the `bucket window + n`, we have run out of budget.
    ///
    /// This is inspired by the generic cell rate algorithm (GCRA) and works
    /// exactly the same as a leaky-bucket.
    pub end: Duration,
}

impl LeakyBucketState {
    pub fn new(now: Duration) -> Self {
        Self { end: now }
    }

    pub fn bucket_is_empty(&self, config: &LeakyBucketConfig, now: Instant) -> bool {
        // if self.end is after now, the bucket is not empty
        config.prev_multiple_of_drain(now - config.epoch) <= self.end
    }

    pub fn tokens(&self, config: &LeakyBucketConfig, now: Instant) -> usize {
        // if self.end is after now, the bucket is not empty
        (config.bucket_width
            - self
                .end
                .saturating_sub(config.prev_multiple_of_drain(now - config.epoch)))
        .div_duration_f64(config.cost) as usize
    }

    /// Immedaitely adds tokens to the bucket, if there is space.
    /// If there is not enough space, no tokens are added. Instead, an error is returned with the time when
    /// there will be space again.
    pub fn add_tokens(
        &mut self,
        config: &LeakyBucketConfig,
        now: Instant,
        n: f64,
    ) -> Result<(), Instant> {
        // round down to the last time we would have drained the bucket.
        let now = config.prev_multiple_of_drain(now - config.epoch);

        let n = config.cost.mul_f64(n);

        let end_plus_n = self.end + n;
        let start_plus_n = end_plus_n.saturating_sub(config.bucket_width);

        //       start          end
        //       |     start+n  |     end+n
        //       |   /          |   /
        // ------{o-[---------o-}--]----o----
        //   now1 ^      now2 ^         ^ now3
        //
        // at now1, the bucket would be completely filled if we add n tokens.
        // at now2, the bucket would be partially filled if we add n tokens.
        // at now3, the bucket would start completely empty before we add n tokens.

        if end_plus_n <= now {
            self.end = now + n;
            Ok(())
        } else if start_plus_n <= now {
            self.end = end_plus_n;
            Ok(())
        } else {
            let ready_at = config.next_multiple_of_drain(start_plus_n);
            Err(config.epoch + ready_at)
        }
    }
}

/// blah blah
pub struct RateLimiter {
    config: LeakyBucketConfig,
    state: Mutex<LeakyBucketState>,

    /// if this rate limiter is fair,
    /// provide a queue to provide this fair ordering.
    queue: Option<Notify>,
}

struct NotifyGuard<'a> {
    notify: &'a Notify,
}

impl Drop for NotifyGuard<'_> {
    fn drop(&mut self) {
        self.notify.notify_one();
    }
}

impl RateLimiter {
    fn steady_rps(&self) -> f64 {
        self.config.cost.as_secs_f64().recip()
    }

    /// Acquire a single permit.
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    ///
    /// # #[tokio::main(flavor="current_thread", start_paused=true)] async fn main() {
    /// let limiter = RateLimiter::builder()
    ///     .initial(10)
    ///     .build();
    ///
    /// limiter.acquire_one().await;
    /// # }
    /// ```
    pub fn acquire_one(&self) -> Acquire {
        self.acquire(1)
    }

    /// Acquire the given number of permits, suspending the current task until
    /// they are available.
    ///
    /// If zero permits are specified, this function never suspends the current
    /// task.
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    ///
    /// # #[tokio::main(flavor="current_thread", start_paused=true)] async fn main() {
    /// let limiter = RateLimiter::builder()
    ///     .initial(10)
    ///     .build();
    ///
    /// limiter.acquire(10).await;
    /// # }
    /// ```
    pub fn acquire(&self, count: usize) -> Acquire {
        let step = self
            .queue
            .as_ref()
            .map(|q| AcquireState::Queue {
                queue: q.notified(),
            })
            .unwrap_or(AcquireState::Idle);

        Acquire {
            inner: self,
            step,
            count: count as f64,
            throttled: false,
        }
    }

    /// Try to acquire the given number of permits, returning `true` if the
    /// given number of permits were successfully acquired.
    ///
    /// If the scheduler is fair, and there are pending tasks waiting to acquire
    /// tokens this method will return `false`.
    ///
    /// If zero permits are specified, this method returns `true`.
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    /// use tokio::time;
    ///
    /// # #[tokio::main(flavor="current_thread", start_paused=true)] async fn main() {
    /// let limiter = RateLimiter::builder().refill(1).initial(1).build();
    ///
    /// assert!(limiter.try_acquire(1));
    /// assert!(!limiter.try_acquire(1));
    /// assert!(limiter.try_acquire(0));
    ///
    /// time::sleep(limiter.interval() * 2).await;
    ///
    /// assert!(limiter.try_acquire(1));
    /// assert!(limiter.try_acquire(1));
    /// assert!(!limiter.try_acquire(1));
    /// # }
    /// ```
    pub fn try_acquire(&self, permits: usize) -> bool {
        if self.config.cost.mul_f64(permits as f64) > self.config.bucket_width {
            return false;
        }

        // check if we are the first in the queue
        let _notify_guard;
        if let Some(queue) = &self.queue {
            let mut notified = std::pin::pin!(queue.notified());
            if !notified.as_mut().enable() {
                return false;
            }

            // notify the next waiter in the queue when we are done.
            _notify_guard = NotifyGuard { notify: queue };
        }

        let now = Instant::now();

        let res = self
            .state
            .lock()
            .add_tokens(&self.config, now, permits as f64);
        match res {
            Ok(()) => true,
            Err(_) => false,
        }
    }

    /// Construct a new [`Builder`] for a [`RateLimiter`].
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    /// use tokio::time::Duration;
    ///
    /// let limiter = RateLimiter::builder()
    ///     .initial(100)
    ///     .refill(100)
    ///     .max(1000)
    ///     .interval(Duration::from_millis(250))
    ///     .fair(false)
    ///     .build();
    /// ```
    pub fn builder() -> Builder {
        Builder::default()
    }

    /// Get the refill amount  of this rate limiter as set through
    /// [`Builder::refill`].
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    ///
    /// let limiter = RateLimiter::builder()
    ///     .refill(1024)
    ///     .build();
    ///
    /// assert_eq!(limiter.refill(), 1024);
    /// ```
    pub fn refill(&self) -> usize {
        self.config
            .drain_interval
            .div_duration_f64(self.config.cost) as usize
    }

    /// Get the refill interval of this rate limiter as set through
    /// [`Builder::interval`].
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    /// use tokio::time::Duration;
    ///
    /// let limiter = RateLimiter::builder()
    ///     .interval(Duration::from_millis(1000))
    ///     .build();
    ///
    /// assert_eq!(limiter.interval(), Duration::from_millis(1000));
    /// ```
    pub fn interval(&self) -> time::Duration {
        self.config.drain_interval
    }

    /// Get the max value of this rate limiter as set through [`Builder::max`].
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    ///
    /// let limiter = RateLimiter::builder()
    ///     .max(1024)
    ///     .build();
    ///
    /// assert_eq!(limiter.max(), 1024);
    /// ```
    pub fn max(&self) -> usize {
        self.config.bucket_width.div_duration_f64(self.config.cost) as usize
    }

    /// Test if the current rate limiter is fair as specified through
    /// [`Builder::fair`].
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    ///
    /// let limiter = RateLimiter::builder()
    ///     .fair(true)
    ///     .build();
    ///
    /// assert_eq!(limiter.is_fair(), true);
    /// ```
    pub fn is_fair(&self) -> bool {
        self.queue.is_some()
    }

    /// Get the current token balance.
    ///
    /// This indicates how many tokens can be requested without blocking.
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    ///
    /// # #[tokio::main(flavor="current_thread", start_paused=true)] async fn main() {
    /// let limiter = RateLimiter::builder()
    ///     .initial(100)
    ///     .build();
    ///
    /// assert_eq!(limiter.balance(), 100);
    /// limiter.acquire(10).await;
    /// assert_eq!(limiter.balance(), 90);
    /// # }
    /// ```
    pub fn balance(&self) -> usize {
        self.state.lock().tokens(&self.config, Instant::now())
    }
}

/// A builder for a [`RateLimiter`].
pub struct Builder {
    /// The max number of tokens.
    max: Option<usize>,
    /// The initial count of tokens.
    initial: usize,
    /// Tokens to add every `per` duration.
    refill: usize,
    /// Interval to add tokens in milliseconds.
    interval: Duration,
    /// If the rate limiter is fair or not.
    fair: bool,
}

impl Builder {
    /// Configure the max number of tokens to use.
    ///
    /// If unspecified, this will default to be 10 times the [`refill`] or the
    /// [`initial`] value, whichever is largest.
    ///
    /// The maximum supported balance is limited to [`isize::MAX`].
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    ///
    /// let limiter = RateLimiter::builder()
    ///     .max(10_000)
    ///     .build();
    /// ```
    ///
    /// [`refill`]: Builder::refill
    /// [`initial`]: Builder::initial
    pub fn max(&mut self, max: usize) -> &mut Self {
        self.max = Some(max);
        self
    }

    /// Configure the initial number of tokens to configure. The default value
    /// is `0`.
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    ///
    /// let limiter = RateLimiter::builder()
    ///     .initial(10)
    ///     .build();
    /// ```
    pub fn initial(&mut self, initial: usize) -> &mut Self {
        self.initial = initial;
        self
    }

    /// Configure the time duration between which we add [`refill`] number to
    /// the bucket rate limiter.
    ///
    /// This is 100ms by default.
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    /// use tokio::time::Duration;
    ///
    /// let limiter = RateLimiter::builder()
    ///     .interval(Duration::from_millis(100))
    ///     .build();
    /// ```
    ///
    /// [`refill`]: Builder::refill
    pub fn interval(&mut self, interval: Duration) -> &mut Self {
        self.interval = interval;
        self
    }

    /// The number of tokens to add at each [`interval`] interval. The default
    /// value is `1`.
    ///
    /// # Panics
    ///
    /// Panics if a refill amount of `0` is specified.
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    ///
    /// let limiter = RateLimiter::builder()
    ///     .refill(100)
    ///     .build();
    /// ```
    ///
    /// [`interval`]: Builder::interval
    pub fn refill(&mut self, refill: usize) -> &mut Self {
        assert!(refill > 0, "refill amount cannot be zero");
        self.refill = refill;
        self
    }

    /// Configure the rate limiter to be fair.
    ///
    /// Fairness is enabled by deafult.
    ///
    /// Fairness ensures that tasks make progress in the order that they acquire
    /// even when the rate limiter is under contention. An unfair scheduler
    /// might have a higher total throughput.
    ///
    /// Fair scheduling also affects the behavior of
    /// [`RateLimiter::try_acquire`] which will return `false` if there are any
    /// pending tasks since they should be given priority.
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    ///
    /// let limiter = RateLimiter::builder()
    ///     .refill(100)
    ///     .fair(false)
    ///     .build();
    /// ```
    pub fn fair(&mut self, fair: bool) -> &mut Self {
        self.fair = fair;
        self
    }

    /// Construct a new [`RateLimiter`].
    ///
    /// # Examples
    ///
    /// ```
    /// use leaky_bucket::RateLimiter;
    /// use tokio::time::Duration;
    ///
    /// let limiter = RateLimiter::builder()
    ///     .refill(100)
    ///     .interval(Duration::from_millis(200))
    ///     .max(10_000)
    ///     .build();
    /// ```
    pub fn build(&self) -> RateLimiter {
        let Self {
            max,
            initial,
            refill,
            interval,
            fair,
        } = *self;

        let initial = initial.min(MAX_BALANCE);
        let refill = refill.min(MAX_BALANCE);

        let max = match max {
            Some(max) => max.min(MAX_BALANCE),
            None => refill
                .max(initial)
                .saturating_mul(DEFAULT_REFILL_MAX_FACTOR)
                .min(MAX_BALANCE),
        };

        // how frequently we drain a single token on average
        let time_cost = interval / refill as u32;
        let bucket_width = time_cost * (max as u32);

        // initial tracks how many tokens are available to put in the bucket
        // we want how many tokens are currently in the bucket
        let initial_tokens = (max - initial) as u32;
        let end = time_cost * initial_tokens;

        RateLimiter {
            config: LeakyBucketConfig {
                epoch: tokio::time::Instant::now(),
                drain_interval: interval,
                cost: time_cost,
                bucket_width,
            },
            state: Mutex::new(LeakyBucketState::new(end)),
            queue: fair.then(|| {
                let queue = Notify::new();
                queue.notify_one();
                queue
            }),
        }
    }
}

/// Construct a new builder with default options.
///
/// # Examples
///
/// ```
/// use leaky_bucket::Builder;
///
/// let limiter = Builder::default().build();
/// ```
impl Default for Builder {
    fn default() -> Self {
        Self {
            fair: true,
            max: None,
            initial: 0,
            refill: 1,
            interval: time::Duration::from_millis(100),
        }
    }
}

pin_project!(
    /// thing
    pub struct Acquire<'a> {
        inner: &'a RateLimiter,
        #[pin]
        step: AcquireState<'a>,
        count: f64,
        throttled: bool,
    }

    impl<'a> PinnedDrop for Acquire<'a> {
        fn drop(this: Pin<&mut Self>) {
            if matches!(this.step, AcquireState::Waiting { .. }) {
                if let Some(queue) = &this.inner.queue {
                    queue.notify_one();
                }
            }
        }
    }
);

pin_project!(
    #[project = AcquireStateProj]
    enum AcquireState<'a> {
        Idle,
        Queue {
            #[pin]
            queue: Notified<'a>,
        },
        Waiting {
            #[pin]
            sleep: Sleep,
        },
        Done,
    }
);

impl Acquire<'_> {
    /// thing
    pub fn is_core(&self) -> bool {
        matches!(self.step, AcquireState::Waiting { .. })
    }
}

impl Future for Acquire<'_> {
    type Output = bool;

    fn poll(
        self: std::pin::Pin<&mut Self>,
        cx: &mut core::task::Context<'_>,
    ) -> Poll<Self::Output> {
        let mut this = self.project();
        if *this.count == 0.0 {
            this.step.set(AcquireState::Done);
            return Poll::Ready(true);
        }

        loop {
            match this.step.as_mut().project() {
                AcquireStateProj::Idle => {}
                AcquireStateProj::Queue { queue } => {
                    if queue.poll(cx).is_pending() {
                        *this.throttled = true;
                        return Poll::Pending;
                    }
                }
                AcquireStateProj::Waiting { sleep } => {
                    if sleep.poll(cx).is_pending() {
                        return Poll::Pending;
                    }
                }
                AcquireStateProj::Done => panic!("polled after completion"),
            }

            let now = tokio::time::Instant::now();
            let res = this
                .inner
                .state
                .lock()
                .add_tokens(&this.inner.config, now, *this.count);

            match res {
                Ok(()) => {
                    if let Some(queue) = &this.inner.queue {
                        queue.notify_one();
                    }
                    this.step.set(AcquireState::Done);
                    return Poll::Ready(*this.throttled);
                }
                Err(ready_at) => {
                    *this.throttled = true;
                    this.step.set(AcquireState::Waiting {
                        sleep: tokio::time::sleep_until(ready_at),
                    });
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{Acquire, RateLimiter};

    fn is_send<T: Send>() {}
    fn is_sync<T: Sync>() {}

    #[test]
    fn assert_send_sync() {
        // is_send::<AcquireOwned>();
        // is_sync::<AcquireOwned>();

        is_send::<RateLimiter>();
        is_sync::<RateLimiter>();

        is_send::<Acquire<'_>>();
        is_sync::<Acquire<'_>>();
    }
}
