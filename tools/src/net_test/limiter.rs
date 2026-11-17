//! The stop of a run and the token bucket that all generators of a run share.

use std::sync::{Condvar, Mutex, MutexGuard};
use std::time::{Duration, Instant, SystemTime};

/// The stop flag of a run: set once, from any thread, and waited on with a timeout, so that every wait of the tool
/// wakes the moment the stop is requested.
#[derive(Default)]
pub struct Stop {
    stopped: Mutex<bool>,
    condition: Condvar,
}

impl Stop {
    fn lock(&self) -> MutexGuard<'_, bool> {
        self.stopped.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub fn is_stopped(&self) -> bool {
        *self.lock()
    }

    /// Sets the flag and wakes every waiter; later calls change nothing.
    pub fn request(&self) {
        *self.lock() = true;
        self.condition.notify_all();
    }

    /// Waits until the stop is requested or the timeout passes; true when the stop was requested.
    pub fn wait_for(&self, timeout: Duration) -> bool {
        let (stopped, _) = self
            .condition
            .wait_timeout_while(self.lock(), timeout, |stopped| !*stopped)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        *stopped
    }

    /// Waits until the stop is requested or the wall clock reaches the time; true when the stop was requested.
    pub fn wait_until(&self, time: SystemTime) -> bool {
        let mut stopped = self.lock();
        loop {
            if *stopped {
                return true;
            }
            let left = match time.duration_since(SystemTime::now()) {
                Ok(left) if !left.is_zero() => left,
                _ => return false,
            };
            stopped = self
                .condition
                .wait_timeout(stopped, left)
                .unwrap_or_else(|poisoned| poisoned.into_inner())
                .0;
        }
    }

    /// Waits until the stop is requested.
    pub fn wait(&self) {
        let _stopped = self
            .condition
            .wait_while(self.lock(), |stopped| !*stopped)
            .unwrap_or_else(|poisoned| poisoned.into_inner());
    }
}

/// A wait shorter than this yields the thread instead of sleeping.
const SPIN_LIMIT: Duration = Duration::from_micros(500);

struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

impl Bucket {
    /// Adds the tokens accrued since the last refill, up to the limit.
    fn refill(&mut self, now: Instant, rate: f64, limit: f64) {
        if now <= self.last_refill {
            return;
        }
        let seconds = (now - self.last_refill).as_secs_f64();
        self.tokens = limit.min(self.tokens + seconds * rate);
        self.last_refill = now;
    }
}

/// The token bucket of the total rate in records per second. The tokens accrue from the start at the rate, up to one
/// second of tokens or one batch, whichever is larger, so a rate below the batch size still releases a batch every
/// batch/rate seconds. A generator takes the tokens of a whole batch before it builds the batch, so the budget never
/// goes into debt. The bucket starts empty.
pub struct RateLimiter<'a> {
    stop: &'a Stop,
    rate: f64,
    capacity: f64,
    bucket: Mutex<Bucket>,
}

impl<'a> RateLimiter<'a> {
    /// The batch is the number of the records that one `acquire` takes; the tokens accrue from `start`.
    pub fn new(records_per_second: i64, batch: i64, stop: &'a Stop, start: Instant) -> Self {
        RateLimiter {
            stop,
            rate: records_per_second as f64,
            capacity: records_per_second.max(batch) as f64,
            bucket: Mutex::new(Bucket {
                tokens: 0.0,
                last_refill: start,
            }),
        }
    }

    fn bucket(&self) -> MutexGuard<'_, Bucket> {
        self.bucket.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// Takes the tokens if the bucket has them now; never waits.
    #[cfg(test)]
    pub fn try_acquire(&self, tokens: i64) -> bool {
        let needed = tokens as f64;
        let mut bucket = self.bucket();
        bucket.refill(Instant::now(), self.rate, self.capacity.max(needed));
        if bucket.tokens < needed {
            return false;
        }
        bucket.tokens -= needed;
        true
    }

    /// Waits until the bucket has the tokens and takes them; false, taking nothing, when the stop comes first.
    pub fn acquire(&self, tokens: i64) -> bool {
        let needed = tokens as f64;
        while !self.stop.is_stopped() {
            let missing = {
                let mut bucket = self.bucket();
                bucket.refill(Instant::now(), self.rate, self.capacity.max(needed));
                if bucket.tokens >= needed {
                    bucket.tokens -= needed;
                    return true;
                }
                needed - bucket.tokens
            };
            let delay = Duration::from_secs_f64(missing / self.rate);
            if delay < SPIN_LIMIT {
                std::thread::yield_now();
            } else if self.stop.wait_for(delay) {
                return false;
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicI64, Ordering};

    use super::*;

    /// Takes batches until the time passes; returns the number taken.
    fn batches_in(limiter: &RateLimiter, batch: i64, time: Duration) -> i64 {
        let end = Instant::now() + time;
        let mut taken = 0;
        while Instant::now() < end {
            if limiter.acquire(batch) && Instant::now() <= end {
                taken += 1;
            }
        }
        taken
    }

    #[test]
    fn ten_thousand_per_second_with_a_batch_of_100_gives_about_100_batches_a_second() {
        let stop = Stop::default();
        let limiter = RateLimiter::new(10_000, 100, &stop, Instant::now());
        let taken = batches_in(&limiter, 100, Duration::from_secs(1));
        assert!((50..=101).contains(&taken), "{taken}");
    }

    #[test]
    fn the_bucket_starts_empty_and_holds_at_most_one_second_of_tokens() {
        let stop = Stop::default();
        // A batch accrues in 10 s.
        let empty = RateLimiter::new(10, 100, &stop, Instant::now());
        assert!(!empty.try_acquire(100));
        let limiter = RateLimiter::new(1_000, 100, &stop, Instant::now());
        std::thread::sleep(Duration::from_millis(1_500));
        // After an idle 1.5 s the bucket holds 1,000 tokens, not 1,500.
        let mut taken = 0;
        while limiter.try_acquire(100) {
            taken += 1;
        }
        assert!((10..=11).contains(&taken), "{taken}");
    }

    #[test]
    fn two_generators_share_the_rate() {
        let stop = Stop::default();
        let limiter = RateLimiter::new(10_000, 100, &stop, Instant::now());
        let total = AtomicI64::new(0);
        std::thread::scope(|scope| {
            for _ in 0..2 {
                scope.spawn(|| total.fetch_add(batches_in(&limiter, 100, Duration::from_secs(1)), Ordering::Relaxed));
            }
        });
        let total = total.load(Ordering::Relaxed);
        assert!((50..=102).contains(&total), "{total}");
    }

    #[test]
    fn a_rate_below_the_batch_releases_a_batch_every_batch_over_rate_seconds() {
        let stop = Stop::default();
        let start = Instant::now();
        let limiter = RateLimiter::new(200, 100, &stop, start);
        assert!(limiter.acquire(100));
        let first = start.elapsed();
        assert!(limiter.acquire(100));
        let second = start.elapsed();
        assert!(first >= Duration::from_millis(450), "{first:?}");
        assert!(second >= Duration::from_millis(950), "{second:?}");
        assert!(second < Duration::from_secs(5), "{second:?}");
    }

    #[test]
    fn a_stop_interrupts_a_waiting_acquire() {
        let stop = Stop::default();
        // A batch every 100 s.
        let limiter = RateLimiter::new(1, 100, &stop, Instant::now());
        let start = Instant::now();
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| limiter.acquire(100));
            std::thread::sleep(Duration::from_millis(100));
            stop.request();
            assert!(!waiter.join().unwrap());
        });
        assert!(start.elapsed() < Duration::from_secs(10));
        assert!(!limiter.acquire(1));
    }

    #[test]
    fn the_stop_wakes_every_wait() {
        let stop = Stop::default();
        assert!(!stop.wait_for(Duration::from_millis(10)));
        assert!(!stop.wait_until(SystemTime::now() + Duration::from_millis(10)));
        assert!(!stop.wait_until(SystemTime::now() - Duration::from_secs(1)));
        std::thread::scope(|scope| {
            scope.spawn(|| {
                std::thread::sleep(Duration::from_millis(50));
                stop.request();
            });
            assert!(stop.wait_until(SystemTime::now() + Duration::from_secs(60)));
        });
        stop.wait();
        assert!(stop.is_stopped());
        assert!(stop.wait_for(Duration::from_secs(60)));
    }
}
