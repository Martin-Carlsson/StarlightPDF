//! Counts each thread's heap allocations, for tests of the memory rule.

use core::alloc::{GlobalAlloc, Layout};
use core::cell::Cell;
use std::alloc::System;

/// The allocator of every test binary linking this crate.
#[global_allocator]
static COUNTING: Counting = Counting;

thread_local! {
    /// The thread's bytes allocated since [`peak`] started and their peak.
    static HELD: Cell<(usize, usize)> = const { Cell::new((0, 0)) };
}

/// [`System`], counting allocations on the allocating thread.
struct Counting;

#[expect(
    unsafe_code,
    reason = "a counting allocator implements an unsafe trait"
)]
// SAFETY: every method passes its arguments to `System` unchanged and only adds bookkeeping that
// never allocates or panics.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc`'s contract, which `System` shares.
        let pointer = unsafe { System.alloc(layout) };
        if !pointer.is_null() {
            grow(layout.size());
        }
        pointer
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::alloc_zeroed`'s contract, which `System` shares.
        let pointer = unsafe { System.alloc_zeroed(layout) };
        if !pointer.is_null() {
            grow(layout.size());
        }
        pointer
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        // SAFETY: the caller upholds `GlobalAlloc::dealloc`'s contract; `ptr` came from
        // `System`, as every allocation here does.
        unsafe {
            System.dealloc(ptr, layout);
        }
        shrink(layout.size());
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        // SAFETY: the caller upholds `GlobalAlloc::realloc`'s contract; `ptr` came from
        // `System`, as every allocation here does.
        let moved = unsafe { System.realloc(ptr, layout, new_size) };
        if !moved.is_null() {
            // Counts old and new as held at once, as a moving reallocation does.
            grow(new_size);
            shrink(layout.size());
        }
        moved
    }
}

/// Adds `size` bytes to the thread's count.
fn grow(size: usize) {
    let _ignored = HELD.try_with(|held| {
        let (current, top) = held.get();
        let now = current.saturating_add(size);
        held.set((now, top.max(now)));
    });
}

/// Removes `size` bytes from the thread's count.
fn shrink(size: usize) {
    let _ignored = HELD.try_with(|held| {
        let (current, top) = held.get();
        held.set((current.saturating_sub(size), top));
    });
}

/// Runs `work` and returns its result and the most bytes it held allocated at once on this
/// thread; frees of earlier allocations count down to zero, never below.
pub fn peak<F, T>(work: F) -> (T, usize)
where
    F: FnOnce() -> T,
{
    HELD.set((0, 0));
    let result = work();
    let (_, top) = HELD.get();
    (result, top)
}
