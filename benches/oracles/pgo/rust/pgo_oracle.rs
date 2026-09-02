#![allow(clippy::missing_safety_doc)]

#[cfg(oracle_case = "branch-layout")]
fn add_path(acc: u64, value: u64) -> u64 {
    acc.wrapping_mul(3).wrapping_add(value)
}
#[cfg(oracle_case = "branch-layout")]
fn subtract_path(acc: u64, value: u64) -> u64 {
    acc.wrapping_mul(5)
        .wrapping_sub(value)
        .wrapping_mul(7)
        .wrapping_add(value)
        .wrapping_mul(3)
        .wrapping_add(11)
}
#[cfg(oracle_case = "branch-layout")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel(
    items: *const u64,
    _items_len: u32,
    n: u32,
    seed: u64,
) -> u64 {
    let mut result = seed;
    for index in 0..n as usize {
        let value = unsafe { *items.add(index) };
        result = if value == 3 {
            add_path(result, value)
        } else {
            subtract_path(result, value)
        };
    }
    result
}

#[cfg(oracle_case = "call-constant-length")]
fn hot_step(acc: u32, value: u32) -> u32 {
    acc.wrapping_mul(3).wrapping_add(value)
}

#[cfg(oracle_case = "call-constant-length")]
fn cold_step(acc: u32, value: u32) -> u32 {
    acc.wrapping_mul(5)
        .wrapping_sub(value)
        .wrapping_mul(7)
        .wrapping_add(value)
        .wrapping_mul(3)
        .wrapping_add(11)
}

#[cfg(oracle_case = "call-constant-length")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel(a: *const u32, _a_len: u32, out: *mut u32, _out_len: u32) {
    let mut acc = 0_u32;
    for i in 0..4000usize {
        let value = unsafe { *a.add(i) };
        acc = if value == 13 {
            hot_step(acc, value)
        } else {
            cold_step(acc, value)
        };
        unsafe { *out.add(i) = acc };
    }
}

#[cfg(oracle_case = "trip-unroll-simd")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel(
    a: *const u32,
    _a_len: u32,
    out: *mut u32,
    _out_len: u32,
    n: u32,
) {
    for i in 0..n as usize {
        unsafe { *out.add(i) = (*a.add(i)).wrapping_add(7) };
    }
}

#[cfg(oracle_case = "memory-bound")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel(
    a: *const u32,
    _a_len: u32,
    b: *const u32,
    _b_len: u32,
    out: *mut u32,
    _out_len: u32,
    n: u32,
) {
    for i in 0..n as usize {
        unsafe { *out.add(i) = (*a.add(i)).wrapping_add(*b.add(i)) };
    }
}

#[cfg(oracle_case = "compute-bound")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel(
    a: *const f64,
    _a_len: u32,
    out: *mut f64,
    _out_len: u32,
    n: u32,
    factor: f64,
) {
    for i in 0..n as usize {
        let value = unsafe { *a.add(i) };
        let mut x = value * factor;
        x += value;
        x *= factor;
        x -= value;
        x *= x;
        x += factor;
        x *= factor;
        x -= value;
        x *= x;
        x += value;
        x *= factor;
        x -= value;
        unsafe { *out.add(i) = x };
    }
}
