//! The stop of a run and the schedule that all generators of a run share.

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

/// The schedule of the specification (section "Rate limit and schedule") that all generators of a run share. The total
/// rate is in records per second and is granted in whole batches: from the first grant `t0`, batch `k`, counted over
/// all generators from 0, is due at `T(k) = t0 + k * batch / rate`. A generator that is ahead sleeps until `T(k)` on
/// the stop condition variable, so it never busy-waits and wakes at the stop. A generator that is behind gets its batch
/// at once and catches up without a bound; no batch is dropped. The first grant sets `t0`, so an idle start gives no
/// burst.
pub struct ScheduleLimiter<'a> {
    stop: &'a Stop,
    /// The length of one batch of the schedule in nanoseconds: `batch * 1e9 / rate`.
    batch_nanos: f64,
    schedule: Mutex<Schedule>,
}

/// The start of the schedule and the number of the next batch.
struct Schedule {
    start: Option<Instant>,
    next: i64,
}

impl<'a> ScheduleLimiter<'a> {
    /// The limiter of `records_per_second` in batches of `batch` records; both are positive.
    pub fn new(records_per_second: i64, batch: i64, stop: &'a Stop) -> Self {
        ScheduleLimiter {
            stop,
            batch_nanos: batch as f64 * 1e9 / records_per_second as f64,
            schedule: Mutex::new(Schedule { start: None, next: 0 }),
        }
    }

    /// The due time `T(k)` of the batch `k` of the schedule that started at `start`.
    pub fn due(&self, start: Instant, k: i64) -> Instant {
        start + Duration::from_nanos((k as f64 * self.batch_nanos).round() as u64)
    }

    /// Takes the next batch of the schedule and waits until it is due; returns its due time `T(k)`, which the caller
    /// takes as the reference of the slip, or `None`, after taking the batch, when the stop comes first. The first
    /// call sets `t0` and returns at once.
    pub fn acquire(&self) -> Option<Instant> {
        self.acquire_with(Instant::now, |left| self.stop.wait_for(left))
    }

    /// `acquire` with the clock and the wait given: the wait sleeps for at most the time left and tells whether the
    /// stop was requested; it may also return early without it.
    fn acquire_with(&self, now: impl Fn() -> Instant, mut wait: impl FnMut(Duration) -> bool) -> Option<Instant> {
        let due = {
            let mut schedule = self.schedule.lock().unwrap_or_else(|poisoned| poisoned.into_inner());
            let start = *schedule.start.get_or_insert_with(&now);
            let k = schedule.next;
            schedule.next += 1;
            self.due(start, k)
        };
        while !self.stop.is_stopped() {
            let left = due.saturating_duration_since(now());
            if left.is_zero() {
                return Some(due);
            }
            if wait(left) {
                break;
            }
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    /// A clock that moves only when a test says so, and the waits it has been asked for.
    struct FakeClock {
        time: Cell<Instant>,
        waits: Cell<Vec<Duration>>,
    }

    impl FakeClock {
        fn new() -> Self {
            FakeClock {
                time: Cell::new(Instant::now()),
                waits: Cell::new(Vec::new()),
            }
        }

        fn now(&self) -> Instant {
            self.time.get()
        }

        fn advance(&self, time: Duration) {
            self.time.set(self.time.get() + time);
        }

        /// A wait that moves the clock by the whole time asked for; the stop never comes.
        fn sleep(&self, left: Duration) -> bool {
            let mut waits = self.waits.take();
            waits.push(left);
            self.waits.set(waits);
            self.advance(left);
            false
        }

        fn wait_count(&self) -> usize {
            let waits = self.waits.take();
            let count = waits.len();
            self.waits.set(waits);
            count
        }
    }

    fn acquire(limiter: &ScheduleLimiter, clock: &FakeClock) -> Option<Instant> {
        limiter.acquire_with(|| clock.now(), |left| clock.sleep(left))
    }

    /// The due times that one generator takes while they are before the end of the time since `t0`.
    fn dues_within(limiter: &ScheduleLimiter, clock: &FakeClock, time: Duration) -> Vec<Instant> {
        let first = acquire(limiter, clock).unwrap();
        let mut dues = vec![first];
        loop {
            let due = acquire(limiter, clock).unwrap();
            if due >= first + time {
                return dues;
            }
            dues.push(due);
        }
    }

    #[test]
    fn ten_thousand_per_second_with_a_batch_of_100_gives_about_100_batches_a_second() {
        let stop = Stop::default();
        let clock = FakeClock::new();
        let limiter = ScheduleLimiter::new(10_000, 100, &stop);
        let dues = dues_within(&limiter, &clock, Duration::from_secs(1));
        assert!((95..=102).contains(&dues.len()), "{}", dues.len());
        // Every batch is due a hundredth of a second after the previous one, and the generator waits for each.
        for pair in dues.windows(2) {
            assert_eq!(pair[1] - pair[0], Duration::from_millis(10));
        }
        assert_eq!(clock.now(), dues[0] + Duration::from_millis(1000));
    }

    #[test]
    fn the_due_time_of_a_batch_is_the_start_plus_batch_over_rate_times_its_number() {
        let stop = Stop::default();
        let limiter = ScheduleLimiter::new(10_000, 100, &stop);
        let start = Instant::now();
        assert_eq!(limiter.due(start, 0), start);
        assert_eq!(limiter.due(start, 250), start + Duration::from_millis(2500));
        // The nanoseconds are rounded: 30 records a second in batches of 10 is a third of a second a batch.
        let third = ScheduleLimiter::new(30, 10, &stop);
        assert_eq!(third.due(start, 1), start + Duration::from_nanos(333_333_333));
        assert_eq!(third.due(start, 2), start + Duration::from_nanos(666_666_667));
        assert_eq!(third.due(start, 3), start + Duration::from_secs(1));
    }

    #[test]
    fn the_first_grant_sets_the_start_so_an_idle_start_gives_no_burst() {
        let stop = Stop::default();
        let clock = FakeClock::new();
        let limiter = ScheduleLimiter::new(1_000, 10, &stop);
        // The limiter stays unused for 300 ms: its time does not count.
        clock.advance(Duration::from_millis(300));
        let first = acquire(&limiter, &clock).unwrap();
        assert_eq!(first, clock.now());
        assert_eq!(clock.wait_count(), 0);
        let dues = dues_within(&limiter, &clock, Duration::from_millis(500));
        // The schedule has batches every 10 ms for 0.5 s after the first grant.
        assert!((45..=52).contains(&(dues.len() + 1)), "{}", dues.len() + 1);
        assert_eq!(clock.wait_count(), dues.len() + 1);
    }

    #[test]
    fn overdue_batches_come_at_once_with_their_due_times() {
        let stop = Stop::default();
        let clock = FakeClock::new();
        // A batch every 10 ms.
        let limiter = ScheduleLimiter::new(100, 1, &stop);
        let start = acquire(&limiter, &clock).unwrap();
        clock.advance(Duration::from_millis(200));
        for k in 1..=20 {
            assert_eq!(acquire(&limiter, &clock), Some(limiter.due(start, k)), "batch {k}");
        }
        assert_eq!(clock.wait_count(), 0, "an overdue batch waits");
        // Batch 21 is due at 210 ms: the generator waits for the 10 ms that are left.
        assert_eq!(acquire(&limiter, &clock), Some(start + Duration::from_millis(210)));
        assert_eq!(clock.waits.take(), [Duration::from_millis(10)]);
    }

    #[test]
    fn a_rate_below_the_batch_waits_batch_over_rate_seconds() {
        let stop = Stop::default();
        let clock = FakeClock::new();
        // 50 records a second in batches of 100: a batch every 2 s.
        let limiter = ScheduleLimiter::new(50, 100, &stop);
        let start = acquire(&limiter, &clock).unwrap();
        assert_eq!(acquire(&limiter, &clock), Some(start + Duration::from_secs(2)));
        assert_eq!(clock.waits.take(), [Duration::from_secs(2)]);
    }

    #[test]
    fn a_wait_that_ends_early_is_repeated_for_the_time_left() {
        let stop = Stop::default();
        let clock = FakeClock::new();
        let limiter = ScheduleLimiter::new(100, 1, &stop);
        let start = acquire(&limiter, &clock).unwrap();
        let mut calls = Vec::new();
        let due = limiter.acquire_with(
            || clock.now(),
            |left| {
                calls.push(left);
                // Two spurious wake-ups, each after a third of the time asked for, then the whole time left.
                clock.advance(if calls.len() < 3 { left / 3 } else { left });
                false
            },
        );
        assert_eq!(due, Some(start + Duration::from_millis(10)));
        assert_eq!(calls.len(), 3);
        assert!(calls.windows(2).all(|pair| pair[1] < pair[0]));
    }

    #[test]
    fn a_stop_during_the_wait_returns_none_and_the_batch_stays_taken() {
        let stop = Stop::default();
        let clock = FakeClock::new();
        let limiter = ScheduleLimiter::new(1, 1, &stop);
        let start = acquire(&limiter, &clock).unwrap();
        assert_eq!(limiter.acquire_with(|| clock.now(), |_| true), None);
        // The stop was taken in the wait: nothing is granted after it, and the batches stay numbered.
        stop.request();
        assert_eq!(limiter.acquire_with(|| clock.now(), |_| false), None);
        let schedule = limiter.schedule.lock().unwrap();
        assert_eq!((schedule.start, schedule.next), (Some(start), 3));
    }

    #[test]
    fn the_batches_are_numbered_over_all_generators() {
        let stop = Stop::default();
        // A schedule of 1 ns a batch: every batch is due at once, whatever the load.
        let limiter = ScheduleLimiter::new(1_000_000_000, 1, &stop);
        let dues = Mutex::new(Vec::new());
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for _ in 0..250 {
                        let due = limiter.acquire().unwrap();
                        dues.lock().unwrap().push(due);
                    }
                });
            }
        });
        let mut dues = dues.into_inner().unwrap();
        dues.sort();
        assert_eq!(dues.len(), 1000);
        let start = dues[0];
        for (k, due) in dues.iter().enumerate() {
            assert_eq!(*due, limiter.due(start, k as i64), "batch {k}");
        }
    }

    #[test]
    fn a_batch_that_is_ahead_of_the_schedule_sleeps_until_it_is_due() {
        let stop = Stop::default();
        // A batch every 100 ms; the lower bound holds on a loaded machine too.
        let limiter = ScheduleLimiter::new(1_000, 100, &stop);
        let begin = Instant::now();
        let first = limiter.acquire().unwrap();
        let second = limiter.acquire().unwrap();
        let third = limiter.acquire().unwrap();
        assert!(begin.elapsed() >= Duration::from_millis(200), "{:?}", begin.elapsed());
        assert_eq!(second - first, Duration::from_millis(100));
        assert_eq!(third - first, Duration::from_millis(200));
    }

    #[test]
    fn a_stop_interrupts_a_waiting_acquire() {
        let stop = Stop::default();
        // A batch every 100 s.
        let limiter = ScheduleLimiter::new(1, 100, &stop);
        assert!(limiter.acquire().is_some());
        let start = Instant::now();
        std::thread::scope(|scope| {
            let waiter = scope.spawn(|| limiter.acquire());
            std::thread::sleep(Duration::from_millis(100));
            stop.request();
            assert_eq!(waiter.join().unwrap(), None);
        });
        assert!(start.elapsed() < Duration::from_secs(10));
        assert_eq!(limiter.acquire(), None);
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
