//! The generator loop of one NetTest instance, the batch of the specification (section "Rate limit and schedule"):
//! wait for the batch of the schedule, read the clock once, fill the batch, publish it, read the elapsed time.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use dxfeed_graal::Event;

use super::events::{EventPool, now_epoch_nanos};
use super::limiter::{ScheduleLimiter, Stop};
use super::statistics::InstanceTotals;

/// The cumulative counters of an instance: written by its generator, read by the reporter. The nanoseconds are
/// weighted by the records: each batch adds records * (the time from the clock read of the batch to the return of the
/// publishing call, so the fill and the call, never the wait of the limiter).
#[derive(Debug, Default)]
pub struct Counters {
    pub records: AtomicI64,
    pub weighted_nanos: AtomicI64,
}

impl Counters {
    /// The values now, for the reporter.
    pub fn totals(&self) -> InstanceTotals {
        InstanceTotals {
            records: self.records.load(Ordering::Relaxed),
            weighted_nanos: self.weighted_nanos.load(Ordering::Relaxed),
        }
    }
}

/// The counters of all generators of a run together: the publishing calls and the schedule slip.
#[derive(Debug, Default)]
pub struct PublishCounters {
    /// The batches published, and the nanoseconds of their fills and publishing calls.
    pub calls: AtomicI64,
    pub nanos: AtomicI64,
    /// With a schedule: the slip of the last batch in nanoseconds, 0 when it was on time.
    pub slip_last_nanos: AtomicI64,
    /// With a schedule: the largest slip since the reporter last took it.
    slip_interval_max_nanos: AtomicI64,
    /// The wall-clock time in nanoseconds since the Unix epoch of the clock read of the first batch of any
    /// generator, 0 before it: the start of the warm-up on the publisher.
    first_batch_epoch_nanos: AtomicI64,
}

impl PublishCounters {
    /// Notes the slip of a batch.
    fn record_slip(&self, slip_nanos: i64) {
        self.slip_last_nanos.store(slip_nanos, Ordering::Relaxed);
        self.slip_interval_max_nanos.fetch_max(slip_nanos, Ordering::Relaxed);
    }

    /// Notes the clock read of a batch; only the first one of the run counts.
    fn record_batch_start(&self, epoch_nanos: i64) {
        let _ = self
            .first_batch_epoch_nanos
            .compare_exchange(0, epoch_nanos, Ordering::Relaxed, Ordering::Relaxed);
    }

    /// The largest slip since the previous call, which starts a new interval.
    pub fn take_interval_slip_max_nanos(&self) -> i64 {
        self.slip_interval_max_nanos.swap(0, Ordering::Relaxed)
    }

    /// The time of the first batch in nanoseconds since the Unix epoch; `None` before it.
    pub fn first_batch_epoch_nanos(&self) -> Option<i64> {
        Some(self.first_batch_epoch_nanos.load(Ordering::Relaxed)).filter(|&time| time != 0)
    }
}

/// What the loop publishes and how.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// The events of a batch.
    pub batch: usize,
    /// Overwrite the batch in place instead of building a new one.
    pub reuse: bool,
}

/// Runs the loop until the stop is requested or `send` fails; a failure requests the stop and is returned. The
/// limiter is shared by all generators; `None` runs flat out, without a schedule and without a slip.
pub fn run_generator<E>(
    pool: &mut EventPool,
    settings: Settings,
    limiter: Option<&ScheduleLimiter>,
    stop: &Stop,
    counters: &Counters,
    publish: &PublishCounters,
    mut send: impl FnMut(&[Event]) -> Result<(), E>,
) -> Result<(), E> {
    let mut batch = Vec::new();
    while !stop.is_stopped() {
        // (1) The wait for the batch of the schedule.
        let due = match limiter {
            Some(limiter) => match limiter.acquire() {
                Some(due) => Some(due),
                None => break,
            },
            None => None,
        };
        // (2) One clock read: the monotonic clock for the slip and the elapsed time, the wall clock for the events.
        let start = Instant::now();
        let epoch_nanos = now_epoch_nanos();
        if let Some(due) = due {
            publish.record_slip(start.saturating_duration_since(due).as_nanos() as i64);
        }
        publish.record_batch_start(epoch_nanos);
        // (3) The fill and (4) the publishing call.
        pool.fill_at(&mut batch, settings.batch, settings.reuse, epoch_nanos);
        let sent = send(&batch);
        // (5) The elapsed time of (3) and (4).
        let elapsed = start.elapsed().as_nanos() as i64;
        if let Err(error) = sent {
            stop.request();
            return Err(error);
        }
        let published = batch.len() as i64;
        counters.records.fetch_add(published, Ordering::Relaxed);
        counters
            .weighted_nanos
            .fetch_add(published * elapsed, Ordering::Relaxed);
        publish.nanos.fetch_add(elapsed, Ordering::Relaxed);
        publish.calls.fetch_add(1, Ordering::Relaxed);
        if !settings.reuse {
            // The copies go now, outside the timed part, as the C tool frees them.
            batch = Vec::new();
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;
    use std::time::Duration;

    use super::*;
    use crate::net_test::cli::EventKind;
    use crate::net_test::symbols::generate_symbols;

    fn pool() -> EventPool {
        EventPool::new(EventKind::Trade, &Arc::new(generate_symbols(10)), 1)
    }

    const SETTINGS: Settings = Settings { batch: 7, reuse: false };

    #[test]
    fn a_successful_publish_counts_the_batches() {
        let stop = Stop::default();
        let counters = Counters::default();
        let publish = PublishCounters::default();
        let mut calls = 0;
        let result: Result<(), ()> = run_generator(&mut pool(), SETTINGS, None, &stop, &counters, &publish, |events| {
            assert_eq!(events.len(), 7);
            std::thread::sleep(Duration::from_millis(1));
            calls += 1;
            if calls == 5 {
                stop.request();
            }
            Ok(())
        });
        assert_eq!(result, Ok(()));
        assert_eq!(calls, 5);
        assert_eq!(counters.records.load(Ordering::Relaxed), 35);
        // Each batch took at least the millisecond of its publish.
        assert!(counters.weighted_nanos.load(Ordering::Relaxed) >= 35 * 1_000_000);
        assert_eq!(publish.calls.load(Ordering::Relaxed), 5);
        assert!(publish.nanos.load(Ordering::Relaxed) >= 5 * 1_000_000);
        // Flat out: no schedule, so no slip.
        assert_eq!(publish.slip_last_nanos.load(Ordering::Relaxed), 0);
        assert_eq!(publish.take_interval_slip_max_nanos(), 0);
    }

    #[test]
    fn a_failed_publish_stops_the_loop_and_returns_the_error() {
        let stop = Stop::default();
        let counters = Counters::default();
        let publish = PublishCounters::default();
        let mut calls = 0;
        let settings = Settings {
            reuse: true,
            ..SETTINGS
        };
        let result = run_generator(&mut pool(), settings, None, &stop, &counters, &publish, |_| {
            calls += 1;
            if calls == 3 { Err("refused") } else { Ok(()) }
        });
        assert_eq!(result, Err("refused"));
        assert_eq!(calls, 3);
        assert!(stop.is_stopped());
        assert_eq!(counters.records.load(Ordering::Relaxed), 14);
        // The failed call is not counted.
        assert_eq!(publish.calls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn the_events_of_the_first_batch_carry_the_clock_read_that_starts_the_warm_up() {
        let stop = Stop::default();
        let counters = Counters::default();
        let publish = PublishCounters::default();
        assert_eq!(publish.first_batch_epoch_nanos(), None);
        let before = now_epoch_nanos();
        let mut first = None;
        let mut calls = 0;
        run_generator::<()>(&mut pool(), SETTINGS, None, &stop, &counters, &publish, |events| {
            calls += 1;
            if calls == 1 {
                let Event::Trade(trade) = &events[0] else { panic!() };
                first = Some((trade.time_sequence, trade.time_nano_part));
            }
            if calls == 3 {
                stop.request();
            }
            Ok(())
        })
        .unwrap();
        let after = now_epoch_nanos();
        let epoch = publish.first_batch_epoch_nanos().unwrap();
        assert!((before..=after).contains(&epoch), "{before} {epoch} {after}");
        // One clock read stamps the batch: its milliseconds and nanosecond part are those of the recorded time.
        let (packed, nano_part) = first.unwrap();
        assert_eq!(((packed >> 32) * 1000) + ((packed >> 22) & 0x3FF), epoch / 1_000_000);
        assert_eq!(i64::from(nano_part), epoch % 1_000_000);
        // Only the first batch of the run sets it.
        assert_eq!(publish.first_batch_epoch_nanos(), Some(epoch));
    }

    #[test]
    fn the_slip_is_the_time_a_batch_is_late_and_the_interval_maximum_restarts() {
        let stop = Stop::default();
        let counters = Counters::default();
        let publish = PublishCounters::default();
        // A batch every 20 ms, but every publishing call takes 30 ms: batch k starts at least 30 ms * k after the
        // start and is due at 20 ms * k, so it is late by at least 10 ms * k, however loaded the machine is.
        let limiter = ScheduleLimiter::new(5_000, 100, &stop);
        let settings = Settings {
            batch: 100,
            reuse: true,
        };
        let mut calls = 0;
        run_generator::<()>(
            &mut pool(),
            settings,
            Some(&limiter),
            &stop,
            &counters,
            &publish,
            |_| {
                std::thread::sleep(Duration::from_millis(30));
                calls += 1;
                if calls == 6 {
                    stop.request();
                }
                Ok(())
            },
        )
        .unwrap();
        let last = publish.slip_last_nanos.load(Ordering::Relaxed);
        assert!(last >= 50_000_000, "{last}");
        let maximum = publish.take_interval_slip_max_nanos();
        assert!(maximum >= last, "{maximum} {last}");
        // The interval maximum starts again; the slip of the last batch stays.
        assert_eq!(publish.take_interval_slip_max_nanos(), 0);
        assert_eq!(publish.slip_last_nanos.load(Ordering::Relaxed), last);
    }

    #[test]
    fn the_slip_records_keep_the_maximum_and_the_last_value() {
        let publish = PublishCounters::default();
        publish.record_slip(5);
        publish.record_slip(9);
        publish.record_slip(2);
        assert_eq!(publish.slip_last_nanos.load(Ordering::Relaxed), 2);
        assert_eq!(publish.take_interval_slip_max_nanos(), 9);
        publish.record_slip(0);
        assert_eq!(publish.slip_last_nanos.load(Ordering::Relaxed), 0);
        assert_eq!(publish.take_interval_slip_max_nanos(), 0);
    }

    #[test]
    fn the_elapsed_time_does_not_include_the_wait_of_the_limiter() {
        let stop = Stop::default();
        let counters = Counters::default();
        let publish = PublishCounters::default();
        // A batch every 250 ms; a publish that returns at once.
        let limiter = ScheduleLimiter::new(400, 100, &stop);
        let settings = Settings {
            batch: 100,
            reuse: false,
        };
        let mut calls = 0;
        run_generator::<()>(
            &mut pool(),
            settings,
            Some(&limiter),
            &stop,
            &counters,
            &publish,
            |_| {
                calls += 1;
                if calls == 3 {
                    stop.request();
                }
                Ok(())
            },
        )
        .unwrap();
        let records = counters.records.load(Ordering::Relaxed);
        assert_eq!(records, 300);
        let average = counters.weighted_nanos.load(Ordering::Relaxed) / records;
        assert!(average < 200_000_000, "{average} ns include the wait");
        assert!(publish.nanos.load(Ordering::Relaxed) < 600_000_000);
    }

    #[test]
    fn a_stop_ends_a_loop_that_waits_on_the_limiter() {
        let stop = Stop::default();
        let counters = Counters::default();
        let publish = PublishCounters::default();
        // A batch every 7 s: the first one goes at once, the loop waits on the limiter for the second.
        let limiter = ScheduleLimiter::new(1, 7, &stop);
        let start = Instant::now();
        std::thread::scope(|scope| {
            let generator = scope.spawn(|| {
                run_generator(
                    &mut pool(),
                    SETTINGS,
                    Some(&limiter),
                    &stop,
                    &counters,
                    &publish,
                    |_| Ok::<(), ()>(()),
                )
            });
            std::thread::sleep(Duration::from_millis(100));
            stop.request();
            assert_eq!(generator.join().unwrap(), Ok(()));
        });
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(counters.records.load(Ordering::Relaxed), 7);
    }
}
