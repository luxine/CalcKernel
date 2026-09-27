/// Multiplies two row-major `n × n` matrices with row-contiguous updates.
///
/// # Safety
/// For positive `n`, `a` and `b` must each point to at least `n * n` readable
/// `f64` values, and `out` must point to at least `n * n` writable values that
/// do not overlap either input. The buffers must be valid for the duration of
/// the call. Non-positive `n` returns without accessing any pointer.
#[no_mangle]
pub unsafe extern "C" fn matmul(a: *const f64, b: *const f64, out: *mut f64, n: i32) {
    if n <= 0 {
        return;
    }

    let side = n as usize;
    let Some(element_count) = side.checked_mul(side) else {
        return;
    };

    // The function's safety contract guarantees valid, non-overlapping buffers.
    let (a, b, out) = unsafe {
        (
            core::slice::from_raw_parts(a, element_count),
            core::slice::from_raw_parts(b, element_count),
            core::slice::from_raw_parts_mut(out, element_count),
        )
    };
    out.fill(0.0);

    for i in 0..side {
        let output_row = i * side;
        for k in 0..side {
            let left = a[output_row + k];
            let right_row = k * side;
            for j in 0..side {
                let output_index = output_row + j;
                out[output_index] += left * b[right_row + j];
            }
        }
    }
}

/// Applies a 3×3 Gaussian kernel using cached row bases. Borders are zero.
///
/// # Safety
/// For positive `n`, `input` must point to at least `n * n` readable `f64`
/// values, and `out` must point to at least `n * n` writable values that do
/// not overlap `input`. The buffers must be valid for the duration of the call.
/// Non-positive `n` returns without accessing either pointer.
#[no_mangle]
pub unsafe extern "C" fn convolve(input: *const f64, out: *mut f64, n: i32) {
    if n <= 0 {
        return;
    }

    let side = n as usize;
    let Some(element_count) = side.checked_mul(side) else {
        return;
    };

    // The function's safety contract guarantees valid, non-overlapping buffers.
    let (input, out) = unsafe {
        (
            core::slice::from_raw_parts(input, element_count),
            core::slice::from_raw_parts_mut(out, element_count),
        )
    };
    out.fill(0.0);

    if side < 3 {
        return;
    }

    for row in 1..side - 1 {
        let top = (row - 1) * side;
        let middle = row * side;
        let bottom = (row + 1) * side;
        let output_row = row * side;

        for column in 1..side - 1 {
            let weighted_sum =
                input[top + column - 1]
                    + 2.0 * input[top + column]
                    + input[top + column + 1]
                    + 2.0 * input[middle + column - 1]
                    + 4.0 * input[middle + column]
                    + 2.0 * input[middle + column + 1]
                    + input[bottom + column - 1]
                    + 2.0 * input[bottom + column]
                    + input[bottom + column + 1];
            out[output_row + column] = weighted_sum / 16.0;
        }
    }
}
