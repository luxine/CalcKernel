"""NumPy kernels used by the cross-language website benchmark.

Inputs and outputs are contiguous row-major float64 arrays, supplied either as
flat n*n buffers or as n-by-n matrices. Each function overwrites all of `out`.
"""

from __future__ import annotations

import numpy as np


_CONVOLVE_SCRATCH: dict[tuple[int, int], tuple[np.ndarray, np.ndarray]] = {}


def _matrix(value: np.ndarray, n: int, name: str, *, writable: bool = False) -> np.ndarray:
    array = np.asarray(value)
    if array.dtype != np.float64:
        raise TypeError(f"{name} must have dtype float64")
    if not array.flags.c_contiguous:
        raise ValueError(f"{name} must be C-contiguous")
    if array.size != n * n:
        raise ValueError(f"{name} must contain n*n values")
    if writable and not array.flags.writeable:
        raise ValueError(f"{name} must be writable")
    return array.reshape((n, n))


def matmul_ordinary(a: np.ndarray, b: np.ndarray, out: np.ndarray, n: int) -> None:
    """Use the standard allocating `@` expression, then copy into `out`."""
    left = _matrix(a, n, "a")
    right = _matrix(b, n, "b")
    result = left @ right
    np.copyto(_matrix(out, n, "out", writable=True), result)


def matmul_tuned(a: np.ndarray, b: np.ndarray, out: np.ndarray, n: int) -> None:
    """Use NumPy's output-buffer API to avoid the result allocation and copy."""
    left = _matrix(a, n, "a")
    right = _matrix(b, n, "b")
    target = _matrix(out, n, "out", writable=True)
    np.matmul(left, right, out=target)


def convolve_ordinary(input: np.ndarray, out: np.ndarray, n: int) -> None:
    """Build the 3x3 Gaussian stencil from vectorized expressions.

    Intermediate arrays are part of this ordinary path's measured work. Pixels
    outside the one-cell interior are zero, matching the other implementations.
    """
    source = _matrix(input, n, "input")
    target = _matrix(out, n, "out", writable=True)
    target.fill(0.0)
    if n < 3:
        return

    acc = source[:-2, :-2] * 1.0
    acc = acc + 2.0 * source[:-2, 1:-1]
    acc = acc + source[:-2, 2:]
    acc = acc + 2.0 * source[1:-1, :-2]
    acc = acc + 4.0 * source[1:-1, 1:-1]
    acc = acc + 2.0 * source[1:-1, 2:]
    acc = acc + source[2:, :-2]
    acc = acc + 2.0 * source[2:, 1:-1]
    acc = acc + source[2:, 2:]
    target[1:-1, 1:-1] = acc / 16.0


def convolve_tuned(input: np.ndarray, out: np.ndarray, n: int) -> None:
    """Apply the same stencil with cached scratch buffers and `out=` ufuncs."""
    source = _matrix(input, n, "input")
    target = _matrix(out, n, "out", writable=True)
    target.fill(0.0)
    if n < 3:
        return

    shape = (n - 2, n - 2)
    scratch = _CONVOLVE_SCRATCH.get(shape)
    if scratch is None:
        scratch = (np.empty(shape, dtype=np.float64), np.empty(shape, dtype=np.float64))
        _CONVOLVE_SCRATCH[shape] = scratch
    acc, term = scratch

    windows = (
        (source[:-2, :-2], 1.0),
        (source[:-2, 1:-1], 2.0),
        (source[:-2, 2:], 1.0),
        (source[1:-1, :-2], 2.0),
        (source[1:-1, 1:-1], 4.0),
        (source[1:-1, 2:], 2.0),
        (source[2:, :-2], 1.0),
        (source[2:, 1:-1], 2.0),
        (source[2:, 2:], 1.0),
    )
    first, factor = windows[0]
    np.multiply(first, factor, out=acc)
    for window, factor in windows[1:]:
        np.multiply(window, factor, out=term)
        np.add(acc, term, out=acc)
    np.divide(acc, 16.0, out=target[1:-1, 1:-1])
