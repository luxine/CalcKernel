use std::{
    alloc::{GlobalAlloc, Layout, System},
    cell::Cell,
};

thread_local! {
    // Const TLS needs neither allocation nor a destructor in allocator callbacks.
    static COUNT: Cell<Option<usize>> = const { Cell::new(None) };
}

struct CountingAllocator;

#[global_allocator]
static ALLOCATOR: CountingAllocator = CountingAllocator;

fn record_allocation() {
    let _ = COUNT.try_with(|count| {
        if let Some(value) = count.get() {
            count.set(Some(value.saturating_add(1)));
        }
    });
}

// SAFETY: All allocation operations preserve System's pointer/layout contracts.
// Recording uses only const TLS and nonpanicking integer operations; it cannot
// allocate, recurse into this allocator, or affect another test thread's count.
unsafe impl GlobalAlloc for CountingAllocator {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: The caller supplies GlobalAlloc's required valid layout.
        unsafe { System.alloc(layout) }
    }

    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        record_allocation();
        // SAFETY: The caller supplies GlobalAlloc's required valid layout.
        unsafe { System.alloc_zeroed(layout) }
    }

    unsafe fn dealloc(&self, pointer: *mut u8, layout: Layout) {
        // SAFETY: The pointer and original layout came from this System wrapper.
        unsafe { System.dealloc(pointer, layout) }
    }

    unsafe fn realloc(&self, pointer: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        record_allocation();
        // SAFETY: The caller preserves GlobalAlloc's pointer/layout/size contract.
        unsafe { System.realloc(pointer, layout, new_size) }
    }
}

struct ResetCount;

impl Drop for ResetCount {
    fn drop(&mut self) {
        COUNT.with(|count| count.set(None));
    }
}

pub(super) fn measure<T>(operation: impl FnOnce() -> T) -> (T, usize) {
    COUNT.with(|count| {
        assert!(
            count.get().is_none(),
            "allocation measurements must not nest"
        );
        count.set(Some(0));
    });
    let reset = ResetCount;
    let output = operation();
    let allocations = COUNT.with(|count| count.get().expect("active measurement"));
    drop(reset);
    (output, allocations)
}

#[test]
fn measurement_should_count_real_allocations_only_inside_its_scope() {
    let (bytes, allocations) = measure(|| std::hint::black_box(vec![7u8; 1024]));
    assert_eq!((bytes.len(), allocations), (1024, 1));
    let (_, next) = measure(|| ());
    assert_eq!(next, 0);
}

#[test]
fn measurement_should_reset_when_the_measured_operation_unwinds() {
    let result = std::panic::catch_unwind(|| measure(|| panic!("measured failure")));
    assert!(result.is_err());
    let (_, allocations) = measure(|| ());
    assert_eq!(allocations, 0);
}

#[test]
fn measurement_should_not_include_allocations_from_another_thread() {
    use std::sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    };

    let flags = Arc::new((AtomicBool::new(false), AtomicBool::new(false)));
    let worker_flags = Arc::clone(&flags);
    let worker = std::thread::spawn(move || {
        while !worker_flags.0.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
        let (bytes, allocations) = measure(|| std::hint::black_box(vec![3u8; 1024]));
        std::hint::black_box(bytes);
        worker_flags.1.store(true, Ordering::Release);
        allocations
    });
    let (_, allocations) = measure(|| {
        flags.0.store(true, Ordering::Release);
        while !flags.1.load(Ordering::Acquire) {
            std::hint::spin_loop();
        }
    });
    assert_eq!((allocations, worker.join().expect("worker")), (0, 1));
}
