use criterion::{black_box, criterion_group, criterion_main, Criterion};
use log_hz::*;
use rayon::prelude::*;
use std::sync::LazyLock;

// Original mutex-based log_hz macro for comparison
macro_rules! log_hz_mutex {
    ($level:expr, $rate:expr, $($arg:tt)+) => {
        {
            static TIC: std::sync::Mutex<Option<std::time::Instant>> = std::sync::Mutex::new(None);
            static INTERVAL: std::sync::LazyLock<std::time::Duration> = std::sync::LazyLock::new(|| std::time::Duration::from_secs_f32(1.0 / ($rate as f32)));
            let mut tick = TIC.lock().expect("log_hz mutex was poisoned, this should never happen");
            match *tick {
                None => {
                    *tick = Some(std::time::Instant::now());
                    log!($level, $($arg)+);
                },
                Some(ref mut tick) => {
                    let now = std::time::Instant::now();
                    let time_since_last = now.duration_since(*tick);
                    if time_since_last > *INTERVAL {
                        *tick = now;
                        log!($level, $($arg)+);
                    }
                }
            }
        }
    }
}

// New lock-free log_hz macro
macro_rules! log_hz_lockfree {
    ($level:expr, $rate:expr, $($arg:tt)+) => {
        {
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::sync::LazyLock;
            use std::time::Instant;

            static START_TIME: LazyLock<Instant> = LazyLock::new(Instant::now);

            static INTERVAL_NS: LazyLock<u64> = LazyLock::new(|| {
                let rate_f64 = $rate as f64;
                if rate_f64 > 0.0 {
                    (1.0 / rate_f64 * 1_000_000_000.0) as u64
                } else {
                    0
                }
            });

            static LAST_LOG_NS: AtomicU64 = AtomicU64::new(0);

            let last_ns = LAST_LOG_NS.load(Ordering::Relaxed);
            let now = Instant::now();
            let elapsed_ns = now.duration_since(*START_TIME).as_nanos() as u64;

            if elapsed_ns.saturating_sub(last_ns) >= *INTERVAL_NS {
                if LAST_LOG_NS.compare_exchange(last_ns, elapsed_ns, Ordering::AcqRel, Ordering::Relaxed).is_ok() {
                    log!($level, $($arg)+);
                }
            }
        }
    };
}

// A second version of the lock-free macro for benchmarking
macro_rules! log_hz_lockfree_v2 {
    ($level:expr, $rate:expr, $($arg:tt)+) => {
        {
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::sync::LazyLock;
            use std::time::Instant;

            static START_TIME: LazyLock<Instant> = LazyLock::new(Instant::now);

            static INTERVAL_NS: LazyLock<u64> = LazyLock::new(|| {
                let rate_f64 = $rate as f64;
                if rate_f64 > 0.0 {
                    (1.0 / rate_f64 * 1_000_000_000.0) as u64
                } else {
                    u64::MAX
                }
            });

            static LAST_LOG_NS: AtomicU64 = AtomicU64::new(0);

            let mut last_ns = LAST_LOG_NS.load(Ordering::Relaxed);
            let now = Instant::now();
            let elapsed_ns = now.duration_since(*START_TIME).as_nanos() as u64;

            if elapsed_ns.saturating_sub(last_ns) >= *INTERVAL_NS {
                loop {
                    match LAST_LOG_NS.compare_exchange_weak(
                        last_ns,
                        elapsed_ns,
                        Ordering::Acquire,
                        Ordering::Relaxed,
                    ) {
                        Ok(_) => {
                            log!($level, $($arg)+);
                            break;
                        }
                        Err(current_ns) => {
                            // Check if another thread already logged
                            if elapsed_ns.saturating_sub(current_ns) < *INTERVAL_NS {
                                break;
                            }
                            last_ns = current_ns;
                        }
                    }
                }
            }
        }
    };
}


// Ultra-fast version using TSC (Time Stamp Counter) via minstant
macro_rules! log_hz_lockfree_v5 {
    ($level:expr, $rate:expr, $($arg:tt)+) => {
        {
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::sync::LazyLock;
            use minstant::Instant;

            static START_TIME: LazyLock<Instant> = LazyLock::new(Instant::now);
            static INTERVAL_NS: LazyLock<u64> = LazyLock::new(|| {
                let rate_f64 = $rate as f64;
                if rate_f64 > 0.0 {
                    (1.0 / rate_f64 * 1_000_000_000.0) as u64
                } else {
                    u64::MAX
                }
            });
            static LAST_LOG_NS: AtomicU64 = AtomicU64::new(0);

            // Use TSC-based timing - ultra fast
            let now = Instant::now();
            let elapsed_ns = now.duration_since(*START_TIME).as_nanos() as u64;
            
            let last_ns = LAST_LOG_NS.load(Ordering::Relaxed);
            
            // Fast path: early exit
            if elapsed_ns.saturating_sub(last_ns) < *INTERVAL_NS {
                return;
            }

            // Single atomic update attempt
            if LAST_LOG_NS.compare_exchange(
                last_ns, 
                elapsed_ns, 
                Ordering::AcqRel, 
                Ordering::Relaxed
            ).is_ok() {
                log!($level, $($arg)+);
            }
        }
    };
}

// Ultra-fast version using coarsetime (CLOCK_MONOTONIC_COARSE on Linux)
macro_rules! log_hz_lockfree_coarsetime {
    ($level:expr, $rate:expr, $($arg:tt)+) => {
        {
            use std::sync::atomic::{AtomicU64, Ordering};
            use std::sync::LazyLock;
            use coarsetime::Instant;

            static START_TIME: LazyLock<Instant> = LazyLock::new(Instant::now);
            static INTERVAL_NS: LazyLock<u64> = LazyLock::new(|| {
                let rate_f64 = $rate as f64;
                if rate_f64 > 0.0 {
                    (1.0 / rate_f64 * 1_000_000_000.0) as u64
                } else {
                    u64::MAX
                }
            });
            static LAST_LOG_NS: AtomicU64 = AtomicU64::new(0);

            // Use coarsetime - ultra fast coarse-grained timing
            let now = Instant::now();
            let elapsed_ns = now.duration_since(*START_TIME).as_nanos() as u64;
            
            let last_ns = LAST_LOG_NS.load(Ordering::Relaxed);
            
            // Fast path: early exit
            if elapsed_ns.saturating_sub(last_ns) < *INTERVAL_NS {
                return;
            }

            // Single atomic update attempt
            if LAST_LOG_NS.compare_exchange(
                last_ns, 
                elapsed_ns, 
                Ordering::AcqRel, 
                Ordering::Relaxed
            ).is_ok() {
                log!($level, $($arg)+);
            }
        }
    };
}

// Mock logger for benchmarking
struct MockLogger;

impl log::Log for MockLogger {
    fn enabled(&self, _metadata: &log::Metadata) -> bool {
        true
    }

    fn log(&self, _record: &log::Record) {
        // Do nothing - we're just measuring the macro overhead
    }

    fn flush(&self) {}
}

// Set up logger once at the beginning
static LOGGER: LazyLock<MockLogger> = LazyLock::new(|| {
    let logger = MockLogger;
    // Try to set the logger, but don't panic if it's already set
    let _ = log::set_logger(Box::leak(Box::new(logger)));
    log::set_max_level(log::LevelFilter::Trace);
    MockLogger
});

fn setup_logger() {
    // Just access the static logger to ensure it's initialized
    let _ = &*LOGGER;
}

// Helper for coarse monotonic time (Linux only)
#[inline(always)]
fn now_monotonic_coarse_ns() -> u64 {
    use libc::{clock_gettime, timespec, CLOCK_MONOTONIC_COARSE};
    let mut ts = timespec { tv_sec: 0, tv_nsec: 0 };
    let ret = unsafe { clock_gettime(CLOCK_MONOTONIC_COARSE, &mut ts) };
    debug_assert_eq!(ret, 0);
    (ts.tv_sec as u64) * 1_000_000_000 + (ts.tv_nsec as u64)
}

fn benchmark_mutex_vs_lockfree(c: &mut Criterion) {
    setup_logger();
    
    let mut group = c.benchmark_group("log_hz_mutex_vs_lockfree");
    
    // Benchmark mutex-based version
    group.bench_function("mutex_version", |b| {
        b.iter(|| {
            log_hz_mutex!(log::Level::Info, 1.0, "Benchmark message {}", black_box(42));
        });
    });
    
    // Benchmark lock-free version
    group.bench_function("lockfree_version", |b| {
        b.iter(|| {
            log_hz_lockfree!(log::Level::Info, 1.0, "Benchmark message {}", black_box(42));
        });
    });

    group.bench_function("lockfree_version_v2", |b| {
        b.iter(|| {
            log_hz_lockfree_v2!(log::Level::Info, 1.0, "Benchmark message {}", black_box(42));
        });
    });

    group.bench_function("lockfree_version_v5", |b| {
        b.iter(|| {
            log_hz_lockfree_v5!(log::Level::Info, 1.0, "Benchmark message {}", black_box(42));
        });
    });

    group.bench_function("lockfree_version_coarsetime", |b| {
        b.iter(|| {
            log_hz_lockfree_coarsetime!(log::Level::Info, 1.0, "Benchmark message {}", black_box(42));
        });
    });
    
    group.finish();
}

fn benchmark_concurrent_access(c: &mut Criterion) {
    setup_logger();
    
    let mut group = c.benchmark_group("concurrent_access");
    
    // Benchmark single-threaded performance
    group.bench_function("single_threaded", |b| {
        b.iter(|| {
            log_hz_lockfree!(log::Level::Info, 1.0, "Single thread message {}", black_box(42));
        });
    });
    
    // Benchmark multi-threaded performance
    group.bench_function("multi_threaded", |b| {
        b.iter(|| {
            (0..100).into_par_iter().for_each(|_| {
                log_hz_lockfree!(log::Level::Info, 1.0, "Multi thread message {}", black_box(42));
            })
        });
    });
    
    group.finish();
}

criterion_group!(
    benches,
    benchmark_mutex_vs_lockfree,
    benchmark_concurrent_access,
);
criterion_main!(benches); 