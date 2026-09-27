#![allow(dead_code, unexpected_cfgs, unused_macros)]

#[cfg(target_arch = "x86_64")]
mod simd {
    use std::arch::x86_64::*;

    #[inline(always)]
    pub unsafe fn map(a: *const u32, out: *mut u32, n: u32, add: u32) {
        let mut i = 0usize;
        let adds = _mm_set1_epi32(add as i32);
        while i + 4 <= n as usize {
            let value = _mm_loadu_si128(a.add(i).cast());
            _mm_storeu_si128(out.add(i).cast(), _mm_add_epi32(value, adds));
            i += 4;
        }
        while i < n as usize {
            *out.add(i) = (*a.add(i)).wrapping_add(add);
            i += 1;
        }
    }

    #[inline(always)]
    pub unsafe fn zip(a: *const u32, b: *const u32, out: *mut u32, n: u32) {
        let mut i = 0usize;
        while i + 4 <= n as usize {
            let left = _mm_loadu_si128(a.add(i).cast());
            let right = _mm_loadu_si128(b.add(i).cast());
            _mm_storeu_si128(out.add(i).cast(), _mm_add_epi32(left, right));
            i += 4;
        }
        while i < n as usize {
            *out.add(i) = (*a.add(i)).wrapping_add(*b.add(i));
            i += 1;
        }
    }

    #[inline(always)]
    pub unsafe fn f64_map(a: *const f64, out: *mut f64, n: u32, factor: f64) {
        let mut i = 0usize;
        let factors = _mm_set1_pd(factor);
        while i + 2 <= n as usize {
            _mm_storeu_pd(out.add(i), _mm_mul_pd(_mm_loadu_pd(a.add(i)), factors));
            i += 2;
        }
        while i < n as usize {
            *out.add(i) = *a.add(i) * factor;
            i += 1;
        }
    }

    #[inline(always)]
    pub unsafe fn cast(a: *const u32, out: *mut f64, n: u32) {
        let mut i = 0usize;
        let zero = _mm_setzero_si128();
        let exponent = _mm_set1_epi64x(0x4330_0000_0000_0000);
        let bias = _mm_set1_pd(4_503_599_627_370_496.0);
        while i + 2 <= n as usize {
            let source = _mm_loadl_epi64(a.add(i).cast());
            let widened = _mm_unpacklo_epi32(source, zero);
            let converted = _mm_sub_pd(
                _mm_castsi128_pd(_mm_or_si128(widened, exponent)),
                bias,
            );
            _mm_storeu_pd(out.add(i), converted);
            i += 2;
        }
        while i < n as usize {
            *out.add(i) = *a.add(i) as f64;
            i += 1;
        }
    }

    #[inline(always)]
    pub unsafe fn reduce(a: *const u32, n: u32) -> u32 {
        let mut i = 0usize;
        let mut lanes = _mm_setzero_si128();
        while i + 4 <= n as usize {
            lanes = _mm_add_epi32(lanes, _mm_loadu_si128(a.add(i).cast()));
            i += 4;
        }
        let values: [u32; 4] = core::mem::transmute(lanes);
        let mut total = values.into_iter().fold(0u32, u32::wrapping_add);
        while i < n as usize {
            total = total.wrapping_add(*a.add(i));
            i += 1;
        }
        total
    }
}

#[cfg(target_arch = "aarch64")]
mod simd {
    use std::arch::aarch64::*;

    #[inline(always)]
    pub unsafe fn map(a: *const u32, out: *mut u32, n: u32, add: u32) {
        let mut i = 0usize;
        let adds = vdupq_n_u32(add);
        while i + 4 <= n as usize {
            let value = vld1q_u32(a.add(i));
            vst1q_u32(out.add(i), vaddq_u32(value, adds));
            i += 4;
        }
        while i < n as usize {
            *out.add(i) = (*a.add(i)).wrapping_add(add);
            i += 1;
        }
    }

    #[inline(always)]
    pub unsafe fn zip(a: *const u32, b: *const u32, out: *mut u32, n: u32) {
        let mut i = 0usize;
        while i + 4 <= n as usize {
            vst1q_u32(
                out.add(i),
                vaddq_u32(vld1q_u32(a.add(i)), vld1q_u32(b.add(i))),
            );
            i += 4;
        }
        while i < n as usize {
            *out.add(i) = (*a.add(i)).wrapping_add(*b.add(i));
            i += 1;
        }
    }

    #[inline(always)]
    pub unsafe fn f64_map(a: *const f64, out: *mut f64, n: u32, factor: f64) {
        let mut i = 0usize;
        let factors = vdupq_n_f64(factor);
        while i + 2 <= n as usize {
            vst1q_f64(out.add(i), vmulq_f64(vld1q_f64(a.add(i)), factors));
            i += 2;
        }
        while i < n as usize {
            *out.add(i) = *a.add(i) * factor;
            i += 1;
        }
    }

    #[inline(always)]
    pub unsafe fn cast(a: *const u32, out: *mut f64, n: u32) {
        let mut i = 0usize;
        while i + 2 <= n as usize {
            let source = vld1_u32(a.add(i));
            vst1q_f64(out.add(i), vcvtq_f64_u64(vmovl_u32(source)));
            i += 2;
        }
        while i < n as usize {
            *out.add(i) = *a.add(i) as f64;
            i += 1;
        }
    }

    #[inline(always)]
    pub unsafe fn reduce(a: *const u32, n: u32) -> u32 {
        let mut i = 0usize;
        let mut lanes = vdupq_n_u32(0);
        while i + 4 <= n as usize {
            lanes = vaddq_u32(lanes, vld1q_u32(a.add(i)));
            i += 4;
        }
        let mut total = vaddvq_u32(lanes);
        while i < n as usize {
            total = total.wrapping_add(*a.add(i));
            i += 1;
        }
        total
    }
}

unsafe fn cast(a: *const u32, out: *mut f64, n: u32) {
    simd::cast(a, out, n);
}

macro_rules! void_result {
    ($body:block) => {{ $body #[cfg(oracle_checked)] { return 0i32; } }};
}

unsafe fn vector_map_result(a: *const u32, out: *mut u32, n: u32, add: u32) -> VoidResult {
    #[cfg(oracle_checked)]
    {
        for index in 0..n as usize {
            let Some(value) = (*a.add(index)).checked_add(add) else {
                return 1;
            };
            *out.add(index) = value;
        }
        0
    }
    #[cfg(not(oracle_checked))]
    {
        simd::map(a, out, n, add);
    }
}

unsafe fn zip_result(a: *const u32, b: *const u32, out: *mut u32, n: u32) -> VoidResult {
    #[cfg(oracle_checked)]
    {
        for index in 0..n as usize {
            let Some(value) = (*a.add(index)).checked_add(*b.add(index)) else {
                return 1;
            };
            *out.add(index) = value;
        }
        0
    }
    #[cfg(not(oracle_checked))]
    {
        simd::zip(a, b, out, n);
    }
}

#[cfg(oracle_case = "map_u32")]
#[no_mangle]
pub unsafe extern "C" fn ck_oracle_kernel(
    a: *const u32,
    _: u32,
    out: *mut u32,
    _: u32,
    n: u32,
) -> VoidResult {
    vector_map_result(a, out, n, 7)
}

#[cfg(oracle_case = "zip_u32")]
#[no_mangle]
pub unsafe extern "C" fn ck_oracle_kernel(
    a: *const u32,
    _: u32,
    b: *const u32,
    _: u32,
    out: *mut u32,
    _: u32,
    n: u32,
) -> VoidResult {
    zip_result(a, b, out, n)
}

#[cfg(oracle_case = "strict_f64")]
#[no_mangle]
pub unsafe extern "C" fn ck_oracle_kernel(
    a: *const f64,
    _: u32,
    out: *mut f64,
    _: u32,
    n: u32,
    factor: f64,
) -> VoidResult {
    void_result!({
        simd::f64_map(a, out, n, factor);
    })
}

#[cfg(oracle_case = "integer_cast")]
#[no_mangle]
pub unsafe extern "C" fn ck_oracle_kernel(
    a: *const u32,
    _: u32,
    out: *mut f64,
    _: u32,
    n: u32,
) -> VoidResult {
    void_result!({
        cast(a, out, n);
    })
}

#[cfg(all(oracle_case = "modular_reduction", not(oracle_checked)))]
#[no_mangle]
pub unsafe extern "C" fn ck_oracle_kernel(a: *const u32, _: u32, n: u32) -> u32 {
    simd::reduce(a, n)
}

#[cfg(all(oracle_case = "modular_reduction", oracle_checked))]
#[no_mangle]
pub unsafe extern "C" fn ck_oracle_kernel(a: *const u32, a_len: u32, n: u32, out: *mut u32) -> i32 {
    let mut total = 0u32;
    for index in 0..n as usize {
        if index >= a_len as usize {
            return 1;
        }
        let Some(next) = total.checked_add(*a.add(index)) else {
            return 1;
        };
        total = next;
    }
    *out = total;
    0
}

#[cfg(oracle_case = "slp_quad")]
#[no_mangle]
pub unsafe extern "C" fn ck_oracle_kernel(
    a: *const u32,
    a_len: u32,
    b: *const u32,
    b_len: u32,
    out: *mut u32,
    out_len: u32,
) -> VoidResult {
    #[cfg(oracle_checked)]
    if a_len < 4 || b_len < 4 || out_len < 4 {
        return 1;
    }
    zip_result(a, b, out, 4)
}

#[cfg(oracle_case = "runtime_noalias")]
#[no_mangle]
pub unsafe extern "C" fn ck_oracle_kernel(
    a: *const u32,
    a_len: u32,
    out: *mut u32,
    out_len: u32,
    n: u32,
) -> VoidResult {
    #[cfg(oracle_checked)]
    {
        for index in 0..n as usize {
            if index >= a_len as usize {
                return 1;
            }
            let Some(value) = (*a.add(index)).checked_add(11) else {
                return 1;
            };
            if index >= out_len as usize {
                return 1;
            }
            *out.add(index) = value;
        }
        return 0;
    }
    #[cfg(not(oracle_checked))]
    vector_map_result(a, out, n, 11)
}

#[cfg(oracle_case = "specialized_length")]
#[no_mangle]
pub unsafe extern "C" fn ck_oracle_kernel(
    a: *const u32,
    _: u32,
    out: *mut u32,
    _: u32,
) -> VoidResult {
    vector_map_result(a, out, 4000, 13)
}

unsafe fn generic_result(a: *const u32, out: *mut u32, n: u32) -> VoidResult {
    for index in 0..n as usize {
        #[cfg(oracle_checked)]
        let Some(value) = (*a.add(index)).checked_add(17) else {
            return 1;
        };
        #[cfg(not(oracle_checked))]
        let value = (*a.add(index)).wrapping_add(17);
        *out.add(index) = value;
    }
    #[cfg(oracle_checked)]
    return 0;
}

#[cfg(oracle_case = "contract_noalias")]
#[no_mangle]
pub unsafe extern "C" fn ck_oracle_kernel(
    a: *const u32,
    _: u32,
    out: *mut u32,
    _: u32,
    n: u32,
) -> VoidResult {
    generic_result(a, out, n)
}

#[cfg(oracle_case = "contract_fixed_length")]
#[no_mangle]
pub unsafe extern "C" fn ck_oracle_kernel(
    a: *const u32,
    _: u32,
    out: *mut u32,
    _: u32,
    n: u32,
) -> VoidResult {
    generic_result(a, out, n)
}

#[cfg(oracle_checked)]
type VoidResult = i32;
#[cfg(not(oracle_checked))]
type VoidResult = ();
