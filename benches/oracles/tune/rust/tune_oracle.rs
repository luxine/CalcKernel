#![allow(clippy::missing_safety_doc)]

#[cfg(tune_case = "branch-layout")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel(items: *const u64, _: u32, n: u32, seed: u64) -> u64 {
    let mut result = seed;
    for index in 0..n as usize {
        let value = unsafe { *items.add(index) };
        result = if value == 3 {
            result.wrapping_mul(3).wrapping_add(value)
        } else {
            result
                .wrapping_mul(5)
                .wrapping_sub(value)
                .wrapping_mul(7)
                .wrapping_add(value)
                .wrapping_mul(3)
                .wrapping_add(11)
        };
    }
    result
}

#[cfg(tune_case = "call-constant-length")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel(a: *const u32, _: u32, out: *mut u32, _: u32) {
    let mut acc = 0_u32;
    for index in 0..4_000 {
        let value = unsafe { *a.add(index) };
        acc = if value == 13 {
            acc.wrapping_mul(3).wrapping_add(value)
        } else {
            acc.wrapping_mul(5)
                .wrapping_sub(value)
                .wrapping_mul(7)
                .wrapping_add(value)
                .wrapping_mul(3)
                .wrapping_add(11)
        };
        unsafe { *out.add(index) = acc };
    }
}

#[cfg(any(
    tune_case = "trip-unroll-simd",
    tune_case = "contract-noalias",
    tune_case = "contract-fixed-length"
))]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel(a: *const u32, _: u32, out: *mut u32, _: u32, n: u32) {
    let addend = if cfg!(tune_case = "trip-unroll-simd") {
        7
    } else {
        17
    };
    for index in 0..n as usize {
        unsafe { *out.add(index) = (*a.add(index)).wrapping_add(addend) };
    }
}

#[cfg(tune_case = "memory-bound")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel(
    a: *const u32,
    _: u32,
    b: *const u32,
    _: u32,
    out: *mut u32,
    _: u32,
    n: u32,
) {
    for index in 0..n as usize {
        unsafe { *out.add(index) = (*a.add(index)).wrapping_add(*b.add(index)) };
    }
}

#[cfg(tune_case = "compute-bound")]
#[unsafe(no_mangle)]
pub unsafe extern "C" fn kernel(
    a: *const f64,
    _: u32,
    out: *mut f64,
    _: u32,
    n: u32,
    factor: f64,
) {
    for index in 0..n as usize {
        let value = unsafe { *a.add(index) };
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
        unsafe { *out.add(index) = x };
    }
}
