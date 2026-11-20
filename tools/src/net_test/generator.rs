//! The generator loop of one NetTest instance: take the tokens of a batch, build the batch, publish it, count it.

use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Instant;

use dxfeed_graal::Event;

use super::events::EventPool;
use super::limiter::{RateLimiter, Stop};

/// The cumulative counters of an instance: written by its generator, read by the reporter. The nanoseconds are
/// weighted by the records: each batch adds records * (the time from the start of its build to the return of the
/// publishing call).
#[derive(Debug, Default)]
pub struct Counters {
    pub records: AtomicI64,
    pub weighted_nanos: AtomicI64,
}

/// What the loop publishes and how.
#[derive(Debug, Clone, Copy)]
pub struct Settings {
    /// The events of a batch.
    pub batch: usize,
    /// Overwrite the batch in place instead of building a new one.
    pub reuse: bool,
}

/// Runs the loop until the stop is requested or `publish` fails; a failure requests the stop and is returned. The
/// limiter is shared by all generators; `None` runs flat out.
pub fn run_generator<E>(
    pool: &mut EventPool,
    settings: Settings,
    limiter: Option<&RateLimiter>,
    stop: &Stop,
    counters: &Counters,
    mut publish: impl FnMut(&[Event]) -> Result<(), E>,
) -> Result<(), E> {
    let records = settings.batch as i64;
    let mut batch = Vec::new();
    while !stop.is_stopped() {
        if limiter.is_some_and(|limiter| !limiter.acquire(records)) {
            break;
        }
        let start = Instant::now();
        pool.fill(&mut batch, settings.batch, settings.reuse);
        let published = publish(&batch);
        let elapsed = start.elapsed().as_nanos() as i64;
        if let Err(error) = published {
            stop.request();
            return Err(error);
        }
        let published = batch.len() as i64;
        counters.records.fetch_add(published, Ordering::Relaxed);
        counters
            .weighted_nanos
            .fetch_add(published * elapsed, Ordering::Relaxed);
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
        let mut calls = 0;
        let result: Result<(), ()> = run_generator(&mut pool(), SETTINGS, None, &stop, &counters, |events| {
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
    }

    #[test]
    fn a_failed_publish_stops_the_loop_and_returns_the_error() {
        let stop = Stop::default();
        let counters = Counters::default();
        let mut calls = 0;
        let settings = Settings {
            reuse: true,
            ..SETTINGS
        };
        let result = run_generator(&mut pool(), settings, None, &stop, &counters, |_| {
            calls += 1;
            if calls == 3 { Err("refused") } else { Ok(()) }
        });
        assert_eq!(result, Err("refused"));
        assert_eq!(calls, 3);
        assert!(stop.is_stopped());
        assert_eq!(counters.records.load(Ordering::Relaxed), 14);
    }

    #[test]
    fn a_stop_ends_a_loop_that_waits_on_the_limiter() {
        let stop = Stop::default();
        let counters = Counters::default();
        // A batch every 7 s: the loop waits on the limiter from the start.
        let limiter = RateLimiter::new(1, 7, &stop, Instant::now());
        let start = Instant::now();
        std::thread::scope(|scope| {
            let generator = scope.spawn(|| {
                run_generator(&mut pool(), SETTINGS, Some(&limiter), &stop, &counters, |_| {
                    Ok::<(), ()>(())
                })
            });
            std::thread::sleep(Duration::from_millis(100));
            stop.request();
            assert_eq!(generator.join().unwrap(), Ok(()));
        });
        assert!(start.elapsed() < Duration::from_secs(5));
        assert_eq!(counters.records.load(Ordering::Relaxed), 0);
    }
}
