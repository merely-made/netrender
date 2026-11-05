// Copyright 2026 Mark Alan Boykin
// SPDX-License-Identifier: MPL-2.0

//! Current-thread user plus system CPU time, never process-wide CPU time.

use std::{thread::ThreadId, time::Duration};

#[derive(Clone, Copy, Debug)]
pub struct ThreadCpuStamp {
    thread: Option<ThreadId>,
    value: Option<Duration>,
}

impl ThreadCpuStamp {
    pub fn now() -> Self {
        #[cfg(target_arch = "wasm32")]
        let thread = None;
        #[cfg(not(target_arch = "wasm32"))]
        let thread = Some(std::thread::current().id());
        Self {
            thread,
            value: current_thread_cpu_time(),
        }
    }

    /// Adjacent boundaries can share stamps. Cross-thread observations,
    /// unavailable clocks and counter regressions remain unavailable.
    pub fn elapsed_since(self, start: Self) -> Option<Duration> {
        if self.thread != start.thread {
            return None;
        }
        self.value?.checked_sub(start.value?)
    }
}

/// Absolute current-thread counter for a synchronous provider's diagnostic
/// hook. Use `ThreadCpuStamp` when retaining boundaries across calls.
pub fn current_thread_cpu_time() -> Option<Duration> {
    platform::read()
}

#[cfg(all(
    target_pointer_width = "64",
    any(target_os = "macos", target_os = "linux")
))]
mod platform {
    use super::Duration;
    use std::ffi::{c_int, c_long};

    #[repr(C)]
    struct Timespec {
        seconds: c_long,
        nanoseconds: c_long,
    }
    unsafe extern "C" {
        fn clock_gettime(clock: c_int, value: *mut Timespec) -> c_int;
        #[cfg(test)]
        fn clock_getres(clock: c_int, value: *mut Timespec) -> c_int;
    }

    #[cfg(target_os = "macos")]
    const CLOCK: c_int = 16;
    #[cfg(target_os = "linux")]
    const CLOCK: c_int = 3;

    #[cfg(test)]
    pub(super) fn resolution() -> Option<Duration> {
        let mut value = Timespec {
            seconds: 0,
            nanoseconds: 0,
        };
        // SAFETY: same validated POSIX ABI as read(), no retained pointer.
        if unsafe { clock_getres(CLOCK, &mut value) } != 0 {
            return None;
        }
        if value.seconds < 0 || !(0..1_000_000_000).contains(&value.nanoseconds) {
            return None;
        }
        Some(Duration::new(
            value.seconds as u64,
            value.nanoseconds as u32,
        ))
    }

    pub(super) fn read() -> Option<Duration> {
        // Darwin SDK _time.h defines CLOCK_THREAD_CPUTIME_ID=16 (10.12+).
        // Linux's POSIX clock ID is3. Both64-bit ABIs use two c_long fields.
        let mut value = Timespec {
            seconds: 0,
            nanoseconds: 0,
        };
        // SAFETY: valid clock ID, writable correctly aligned repr(C)timespec;
        // libc retains no pointer. Failure and invalid values are unavailable.
        if unsafe { clock_gettime(CLOCK, &mut value) } != 0 {
            return None;
        }
        if value.seconds < 0 || !(0..1_000_000_000).contains(&value.nanoseconds) {
            return None;
        }
        Some(Duration::new(
            value.seconds as u64,
            value.nanoseconds as u32,
        ))
    }
}

#[cfg(not(all(
    target_pointer_width = "64",
    any(target_os = "macos", target_os = "linux")
)))]
mod platform {
    pub(super) fn read() -> Option<std::time::Duration> {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(all(
        target_pointer_width = "64",
        any(target_os = "macos", target_os = "linux")
    ))]
    #[test]
    fn native_clock_is_available_and_sleep_is_not_counted_as_thread_cpu() {
        let wall = std::time::Instant::now();
        let start = ThreadCpuStamp::now();
        let mut value = 7u64;
        for n in 1..100_000 {
            value = std::hint::black_box(value.wrapping_mul(n));
        }
        std::hint::black_box(value);
        std::thread::sleep(Duration::from_millis(20));
        let cpu = ThreadCpuStamp::now()
            .elapsed_since(start)
            .expect("supported native thread CPU clock");
        let elapsed = wall.elapsed();
        eprintln!(
            "threadCPUclock=POSIX CLOCK_THREAD_CPUTIME_ID os={} resolution_ns={:?} wall_ns={} cpu_ns={}",
            std::env::consts::OS,
            platform::resolution().map(|v| v.as_nanos()),
            elapsed.as_nanos(),
            cpu.as_nanos()
        );
        assert!(cpu > Duration::ZERO);
        assert!(elapsed >= Duration::from_millis(20));
        assert!(cpu < elapsed, "sleep must not count as current-thread CPU");
    }

    #[test]
    fn unavailable_regressing_and_cross_thread_stamps_do_not_become_zero_cpu() {
        let thread = Some(std::thread::current().id());
        let start = ThreadCpuStamp {
            thread,
            value: Some(Duration::from_nanos(5)),
        };
        assert_eq!(
            ThreadCpuStamp {
                thread,
                value: Some(Duration::from_nanos(8))
            }
            .elapsed_since(start),
            Some(Duration::from_nanos(3))
        );
        assert_eq!(
            ThreadCpuStamp {
                thread,
                value: Some(Duration::from_nanos(4))
            }
            .elapsed_since(start),
            None
        );
        assert_eq!(
            ThreadCpuStamp {
                thread,
                value: None
            }
            .elapsed_since(start),
            None
        );
        assert_eq!(
            std::thread::spawn(move || ThreadCpuStamp::now().elapsed_since(start))
                .join()
                .unwrap(),
            None
        );
    }

    #[test]
    fn adjacent_cpu_boundaries_telescope_on_the_same_thread() {
        let a = ThreadCpuStamp::now();
        let mut value = 1u64;
        for n in 1..10000 {
            value = std::hint::black_box(value.wrapping_mul(n));
        }
        let b = ThreadCpuStamp::now();
        std::hint::black_box(value);
        let c = ThreadCpuStamp::now();
        if let (Some(ab), Some(bc), Some(ac)) =
            (b.elapsed_since(a), c.elapsed_since(b), c.elapsed_since(a))
        {
            assert_eq!(ab + bc, ac);
        } else {
            assert!(current_thread_cpu_time().is_none());
        }
    }
}
