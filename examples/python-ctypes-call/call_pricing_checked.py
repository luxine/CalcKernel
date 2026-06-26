from __future__ import annotations

import ctypes
import platform
from pathlib import Path


CK_Status = ctypes.c_int32
CK_OK = 0
CK_ERR_OVERFLOW = 1
CK_ERR_DIV_BY_ZERO = 2
CK_ERR_NULL_POINTER = 3


class Item(ctypes.Structure):
    _fields_ = [
        ("price", ctypes.c_int64),
        ("qty", ctypes.c_int64),
        ("discount", ctypes.c_int64),
        ("tax_rate_ppm", ctypes.c_int64),
    ]


def library_path() -> Path:
    repo_root = Path(__file__).resolve().parents[2]
    system = platform.system()

    if system == "Darwin":
        file_name = "libpricing_checked.dylib"
    elif system == "Linux":
        file_name = "libpricing_checked.so"
    elif system == "Windows":
        file_name = "pricing_checked.dll"
    else:
        raise RuntimeError(f"unsupported platform: {system}")

    return repo_root / "build" / file_name


def configure_library(path: Path) -> ctypes.CDLL:
    lib = ctypes.CDLL(str(path))
    lib.calc_items.argtypes = [
        ctypes.POINTER(Item),
        ctypes.c_int32,
        ctypes.POINTER(ctypes.c_int64),
        ctypes.POINTER(ctypes.c_int32),
    ]
    lib.calc_items.restype = CK_Status
    return lib


def run_success_case(lib: ctypes.CDLL) -> None:
    items = (Item * 3)(
        Item(price=10000, qty=2, discount=1000, tax_rate_ppm=82500),
        Item(price=2500, qty=4, discount=0, tax_rate_ppm=100000),
        Item(price=1200, qty=5, discount=500, tax_rate_ppm=100000),
    )
    out = (ctypes.c_int64 * len(items))(0, 0, 0)
    ik_return = ctypes.c_int32(-1)

    status = lib.calc_items(items, ctypes.c_int32(len(items)), out, ctypes.byref(ik_return))
    if status != CK_OK:
        raise RuntimeError(f"calc_items returned CK_Status {status}")

    if ik_return.value != 0:
        raise AssertionError(f"unexpected ik_return: expected 0, got {ik_return.value}")

    expected = [20567, 11000, 6050]
    actual = list(out)
    if actual != expected:
        raise AssertionError(f"unexpected output: expected {expected}, got {actual}")

    print("OK")


def run_overflow_case(lib: ctypes.CDLL) -> None:
    items = (Item * 1)(
        Item(price=ctypes.c_int64(9223372036854775807).value, qty=2, discount=0, tax_rate_ppm=0),
    )
    out = (ctypes.c_int64 * len(items))(0)
    ik_return = ctypes.c_int32(-1)

    status = lib.calc_items(items, ctypes.c_int32(len(items)), out, ctypes.byref(ik_return))
    if status != CK_ERR_OVERFLOW:
        raise AssertionError(f"expected CK_ERR_OVERFLOW, got CK_Status {status}")

    print("overflow check OK")


def main() -> None:
    path = library_path()
    if not path.exists():
        raise FileNotFoundError(
            f"dynamic library not found: {path}\n"
            "Build it first with `pnpm ckc build examples/pricing.ck --out build/libpricing_checked --overflow checked` "
            "on macOS/Linux, or `pnpm ckc build examples/pricing.ck --out build/pricing_checked.dll --overflow checked` on Windows."
        )

    lib = configure_library(path)
    run_success_case(lib)
    run_overflow_case(lib)


if __name__ == "__main__":
    main()
