//! The runtime statistics of the isolate and its process.

mod support;

use std::time::Duration;

use dxfeed_graal::system;
use support::{is_child, run_child};

#[test]
fn the_serial_gc_gives_the_heap_the_collections_and_the_threads() {
    let stats = system::get_runtime_statistics().unwrap();
    let used = stats.heap_used_bytes.expect("heap_used_bytes");
    let committed = stats.heap_committed_bytes.expect("heap_committed_bytes");
    let max = stats.heap_max_bytes.expect("heap_max_bytes");
    // The values are read one by one while other tests attach threads and allocate, so only the bounds that hold
    // across those reads are checked: the maximum of the heap does not change.
    assert!(
        used > 0 && used <= max && 0 < committed && committed <= max,
        "{stats:?}"
    );
    assert!(
        stats.young_gc_count.is_some() && stats.young_gc_time.is_some(),
        "{stats:?}"
    );
    assert!(
        stats.full_gc_count.is_some() && stats.full_gc_time.is_some(),
        "{stats:?}"
    );
    for pool in [stats.eden_used_bytes, stats.survivor_used_bytes, stats.old_used_bytes] {
        assert!(pool.is_some(), "{stats:?}");
    }
    for threads in [
        stats.thread_count,
        stats.peak_thread_count,
        stats.daemon_thread_count,
        stats.total_started_thread_count,
    ] {
        assert!(threads.is_some_and(|n| n >= 1), "{stats:?}");
    }
    assert!(
        stats.available_processors.expect("available_processors") >= 1,
        "{stats:?}"
    );
    assert!(stats.process_cpu_time.is_some() && stats.uptime.is_some(), "{stats:?}");
}

#[test]
fn the_counters_do_not_go_back() {
    let first = system::get_runtime_statistics().unwrap();
    // Allocate in the isolate, so it has something to count.
    for _ in 0..100 {
        let _ = system::get_property("java.version").unwrap();
    }
    std::thread::sleep(Duration::from_millis(50));
    let second = system::get_runtime_statistics().unwrap();
    let grows = |a: Option<u64>, b: Option<u64>| a.unwrap() <= b.unwrap();
    let grows_time = |a: Option<Duration>, b: Option<Duration>| a.unwrap() <= b.unwrap();
    assert!(
        grows(first.young_gc_count, second.young_gc_count),
        "{first:?} {second:?}"
    );
    assert!(grows(first.full_gc_count, second.full_gc_count), "{first:?} {second:?}");
    assert!(grows(
        first.total_started_thread_count,
        second.total_started_thread_count
    ));
    assert!(grows_time(first.young_gc_time, second.young_gc_time));
    assert!(grows_time(first.full_gc_time, second.full_gc_time));
    assert!(grows_time(first.process_cpu_time, second.process_cpu_time));
    assert!(first.uptime.unwrap() < second.uptime.unwrap(), "{first:?} {second:?}");
}

#[test]
fn the_heap_size_follows_the_isolate_arguments() {
    run_child("child_the_heap_size_follows_the_isolate_arguments");
}

#[test]
#[ignore = "run by the_heap_size_follows_the_isolate_arguments"]
fn child_the_heap_size_follows_the_isolate_arguments() {
    if !is_child() {
        return;
    }
    system::set_isolate_arguments(["-Xmx96m"]).unwrap();
    let max = system::get_runtime_statistics().unwrap().heap_max_bytes.unwrap();
    assert!(max <= 96 << 20 && max > 48 << 20, "{max}");
}

#[test]
fn the_statistics_work_after_the_shutdown() {
    run_child("child_the_statistics_work_after_the_shutdown");
}

#[test]
#[ignore = "run by the_statistics_work_after_the_shutdown"]
fn child_the_statistics_work_after_the_shutdown() {
    if !is_child() {
        return;
    }
    let before = system::get_runtime_statistics().unwrap();
    system::shutdown().unwrap();
    let after = system::get_runtime_statistics().unwrap();
    assert!(before.uptime.unwrap() <= after.uptime.unwrap());
}
