from __future__ import annotations

import ctypes
import platform
from pathlib import Path


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
        file_name = "libpricing.dylib"
    elif system == "Linux":
        file_name = "libpricing.so"
    elif system == "Windows":
        file_name = "pricing.dll"
    else:
        raise RuntimeError(f"unsupported platform: {system}")

    return repo_root / "build" / file_name


def main() -> None:
    path = library_path()
    if not path.exists():
        raise FileNotFoundError(
            f"dynamic library not found: {path}\n"
            "Build it first with `pnpm ckc build examples/pricing.ck --out build/libpricing` "
            "on macOS/Linux, or `pnpm ckc build examples/pricing.ck --out build/pricing.dll` on Windows."
        )

    lib = ctypes.CDLL(str(path))
    lib.calc_items.argtypes = [
        ctypes.POINTER(Item),
        ctypes.c_int32,
        ctypes.POINTER(ctypes.c_int64),
    ]
    lib.calc_items.restype = ctypes.c_int32

    items = (Item * 3)(
        Item(price=10000, qty=2, discount=1000, tax_rate_ppm=82500),
        Item(price=2500, qty=4, discount=0, tax_rate_ppm=100000),
        Item(price=1200, qty=5, discount=500, tax_rate_ppm=100000),
    )
    out = (ctypes.c_int64 * len(items))(0, 0, 0)

    status = lib.calc_items(items, ctypes.c_int32(len(items)), out)
    if status != 0:
        raise RuntimeError(f"calc_items returned status {status}")

    expected = [20567, 11000, 6050]
    actual = list(out)
    if actual != expected:
        raise AssertionError(f"unexpected output: expected {expected}, got {actual}")

    print("OK")


if __name__ == "__main__":
    main()
