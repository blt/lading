//! Allocation detection system for lading hot paths.
//!
//! This crate provides a mechanism to detect allocations in critical code paths
//! during debug builds. Lading's core design principle is being "strictly faster
//! than targets" - hot paths must not allocate. This crate helps enforce that
//! invariant.
//!
//! The system is compiled out in release builds unless the `alloc-guard` feature
//! is explicitly enabled. In debug builds or when the feature is enabled:
//!
//! - `GuardedAllocator` wraps the system allocator and tracks allocations
//! - `NoAllocGuard` marks code regions where allocations are violations
//! - `check_no_alloc` provides a convenient way to verify no-alloc invariants
//!
//! # Example
//!
//! ```ignore
//! use lading_alloc::{NoAllocGuard, check_no_alloc};
//!
//! // Using RAII guard
//! fn hot_path() {
//!     let _guard = NoAllocGuard::new();
//!     // Any allocation here will be counted as a violation
//! }
//!
//! // Using check helper
//! check_no_alloc(|| {
//!     // Code that should not allocate
//! });
//! ```

#![deny(clippy::cargo)]
#![allow(clippy::multiple_crate_versions)]

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicU64, Ordering};

// Thread-local flag indicating whether allocations are currently guarded.
// When true, any allocation is considered a violation.
#[cfg(any(debug_assertions, feature = "alloc-guard"))]
thread_local! {
    static GUARD_ACTIVE: Cell<bool> = const { Cell::new(false) };
}

// Global counter for allocation violations.
// This is incremented whenever an allocation occurs while a guard is active.
#[cfg(any(debug_assertions, feature = "alloc-guard"))]
static VIOLATION_COUNT: AtomicU64 = AtomicU64::new(0);

/// A global allocator that detects allocations in guarded regions.
///
/// This allocator wraps the system allocator and checks whether allocations
/// occur while a `NoAllocGuard` is active. When they do, it increments the
/// violation counter.
///
/// In release builds without the `alloc-guard` feature, this is a zero-cost
/// wrapper around the system allocator.
#[derive(Debug, Clone, Copy)]
pub struct GuardedAllocator;

unsafe impl GlobalAlloc for GuardedAllocator {
    #[inline]
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        #[cfg(any(debug_assertions, feature = "alloc-guard"))]
        {
            GUARD_ACTIVE.with(|active| {
                if active.get() {
                    VIOLATION_COUNT.fetch_add(1, Ordering::Relaxed);
                }
            });
        }

        // SAFETY: We delegate to the system allocator which handles the
        // allocation safely. The caller is responsible for ensuring the
        // layout is valid.
        unsafe { System.alloc(layout) }
    }

    #[inline]
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // Deallocations are not counted as violations since they are often
        // unavoidable (e.g., dropping temporaries created before the guard).
        // SAFETY: We delegate to the system allocator. The caller ensures
        // ptr was allocated by this allocator with the given layout.
        unsafe { System.dealloc(ptr, layout) }
    }

    #[inline]
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        #[cfg(any(debug_assertions, feature = "alloc-guard"))]
        {
            GUARD_ACTIVE.with(|active| {
                if active.get() {
                    VIOLATION_COUNT.fetch_add(1, Ordering::Relaxed);
                }
            });
        }

        // SAFETY: We delegate to the system allocator.
        unsafe { System.alloc_zeroed(layout) }
    }

    #[inline]
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        #[cfg(any(debug_assertions, feature = "alloc-guard"))]
        {
            GUARD_ACTIVE.with(|active| {
                if active.get() {
                    VIOLATION_COUNT.fetch_add(1, Ordering::Relaxed);
                }
            });
        }

        // SAFETY: We delegate to the system allocator.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

/// RAII guard that marks a no-allocation zone.
///
/// While this guard is held, any allocation on the current thread will be
/// counted as a violation. This is useful for marking hot paths that should
/// not allocate.
///
/// The guard is zero-sized in release builds without the `alloc-guard` feature.
///
/// # Example
///
/// ```ignore
/// use lading_alloc::NoAllocGuard;
///
/// fn hot_path() {
///     let _guard = NoAllocGuard::new();
///     // Allocations here are violations
/// }
/// // Guard dropped, allocations allowed again
/// ```
#[derive(Debug)]
#[must_use = "guard must be held to prevent allocations from being tracked"]
pub struct NoAllocGuard {
    /// Whether the guard was previously active when this guard was created.
    /// We use this to support nested guards correctly.
    #[cfg(any(debug_assertions, feature = "alloc-guard"))]
    was_active: bool,
}

impl NoAllocGuard {
    /// Create a new no-allocation guard.
    ///
    /// While this guard exists, allocations on the current thread are tracked
    /// as violations.
    #[inline]
    pub fn new() -> Self {
        #[cfg(any(debug_assertions, feature = "alloc-guard"))]
        {
            let was_active = GUARD_ACTIVE.with(|active| {
                let prev = active.get();
                active.set(true);
                prev
            });
            Self { was_active }
        }

        #[cfg(not(any(debug_assertions, feature = "alloc-guard")))]
        {
            Self {}
        }
    }
}

impl Default for NoAllocGuard {
    #[inline]
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for NoAllocGuard {
    #[inline]
    fn drop(&mut self) {
        #[cfg(any(debug_assertions, feature = "alloc-guard"))]
        {
            GUARD_ACTIVE.with(|active| {
                active.set(self.was_active);
            });
        }
    }
}

/// Get the current count of allocation violations.
///
/// This returns the total number of allocations that occurred while a
/// `NoAllocGuard` was active since the program started.
///
/// In release builds without the `alloc-guard` feature, this always returns 0.
#[inline]
#[must_use]
pub fn allocation_violations() -> u64 {
    #[cfg(any(debug_assertions, feature = "alloc-guard"))]
    {
        VIOLATION_COUNT.load(Ordering::Relaxed)
    }

    #[cfg(not(any(debug_assertions, feature = "alloc-guard")))]
    {
        0
    }
}

/// Reset the violation counter to zero.
///
/// This is useful for testing where you want to check violations in isolated
/// code sections.
///
/// In release builds without the `alloc-guard` feature, this is a no-op.
#[inline]
pub fn reset_violations() {
    #[cfg(any(debug_assertions, feature = "alloc-guard"))]
    {
        VIOLATION_COUNT.store(0, Ordering::Relaxed);
    }
}

/// Execute a closure and verify no allocations occurred.
///
/// This is a convenience function for testing no-allocation invariants.
/// It resets the violation counter, runs the closure with a guard active,
/// and returns the number of violations that occurred.
///
/// # Example
///
/// ```ignore
/// use lading_alloc::check_no_alloc;
///
/// let violations = check_no_alloc(|| {
///     // Code that should not allocate
///     let x = 1 + 2;
///     x
/// });
/// assert_eq!(violations, 0, "hot path allocated!");
/// ```
#[inline]
pub fn check_no_alloc<F, R>(f: F) -> u64
where
    F: FnOnce() -> R,
{
    #[cfg(any(debug_assertions, feature = "alloc-guard"))]
    {
        let before = allocation_violations();
        let _guard = NoAllocGuard::new();
        // We intentionally drop the result here to ensure any allocations
        // from the return value are counted
        let _ = std::hint::black_box(f());
        allocation_violations() - before
    }

    #[cfg(not(any(debug_assertions, feature = "alloc-guard")))]
    {
        let _ = f();
        0
    }
}

/// Check whether allocation tracking is currently enabled.
///
/// Returns true if we're in a debug build or the `alloc-guard` feature is
/// enabled.
#[inline]
#[must_use]
pub const fn is_tracking_enabled() -> bool {
    cfg!(any(debug_assertions, feature = "alloc-guard"))
}

#[cfg(test)]
mod tests {
    use super::*;

    // Note: These tests only work when the allocator is installed globally.
    // When running as part of the test suite, the global allocator may not
    // be our GuardedAllocator, so we test the logic in isolation.

    #[test]
    fn guard_activates_and_deactivates() {
        #[cfg(any(debug_assertions, feature = "alloc-guard"))]
        {
            // Initially not active
            let was_active = GUARD_ACTIVE.with(|a| a.get());
            assert!(!was_active, "Guard should not be active initially");

            {
                let _guard = NoAllocGuard::new();
                let is_active = GUARD_ACTIVE.with(|a| a.get());
                assert!(is_active, "Guard should be active while held");
            }

            // After drop, not active
            let is_active = GUARD_ACTIVE.with(|a| a.get());
            assert!(!is_active, "Guard should not be active after drop");
        }
    }

    #[test]
    fn nested_guards_work_correctly() {
        #[cfg(any(debug_assertions, feature = "alloc-guard"))]
        {
            {
                let _outer = NoAllocGuard::new();
                assert!(GUARD_ACTIVE.with(|a| a.get()));

                {
                    let _inner = NoAllocGuard::new();
                    assert!(GUARD_ACTIVE.with(|a| a.get()));
                }

                // Still active after inner drops
                assert!(GUARD_ACTIVE.with(|a| a.get()));
            }

            // Not active after outer drops
            assert!(!GUARD_ACTIVE.with(|a| a.get()));
        }
    }

    #[test]
    fn violation_counter_increments() {
        #[cfg(any(debug_assertions, feature = "alloc-guard"))]
        {
            reset_violations();
            let before = allocation_violations();

            // Simulate a violation by directly incrementing
            VIOLATION_COUNT.fetch_add(1, Ordering::Relaxed);

            let after = allocation_violations();
            assert_eq!(after - before, 1);
        }
    }

    #[test]
    fn reset_violations_works() {
        #[cfg(any(debug_assertions, feature = "alloc-guard"))]
        {
            VIOLATION_COUNT.fetch_add(10, Ordering::Relaxed);
            reset_violations();
            assert_eq!(allocation_violations(), 0);
        }
    }

    #[test]
    fn is_tracking_enabled_correct() {
        #[cfg(any(debug_assertions, feature = "alloc-guard"))]
        {
            assert!(is_tracking_enabled());
        }

        #[cfg(not(any(debug_assertions, feature = "alloc-guard")))]
        {
            assert!(!is_tracking_enabled());
        }
    }
}
