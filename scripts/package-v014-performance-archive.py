#!/usr/bin/python3
"""Build the deterministic three-member CK 0.14 performance archive."""

from __future__ import annotations

import argparse
import gzip
import io
import pathlib
import tarfile


def regular_file(path: pathlib.Path) -> bytes:
    metadata = path.lstat()
    if path.is_symlink() or not path.is_file() or metadata.st_size <= 0:
        raise ValueError(f"archive input is not a nonempty regular file: {path}")
    return path.read_bytes()


def build(compiler: pathlib.Path, license_file: pathlib.Path,
          notices: pathlib.Path, output: pathlib.Path) -> None:
    members = sorted([
        ("ckc-v0.14/LICENSE", license_file, 0o644),
        ("ckc-v0.14/THIRD_PARTY_NOTICES.md", notices, 0o644),
        ("ckc-v0.14/ckc", compiler, 0o755),
    ])
    buffer = io.BytesIO()
    with tarfile.open(fileobj=buffer, mode="w", format=tarfile.PAX_FORMAT) as archive:
        for name, source, mode in members:
            data = regular_file(source)
            record = tarfile.TarInfo(name)
            record.size = len(data)
            record.mode = mode
            record.mtime = 0
            record.uid = 0
            record.gid = 0
            record.uname = ""
            record.gname = ""
            record.pax_headers = {}
            archive.addfile(record, io.BytesIO(data))
    output.parent.mkdir(parents=True, exist_ok=True)
    with output.open("xb") as raw:
        with gzip.GzipFile(filename="", mode="wb", fileobj=raw,
                           compresslevel=9, mtime=0) as compressed:
            compressed.write(buffer.getvalue())


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--compiler", type=pathlib.Path, required=True)
    parser.add_argument("--license", dest="license_file", type=pathlib.Path, required=True)
    parser.add_argument("--notices", type=pathlib.Path, required=True)
    parser.add_argument("--out", type=pathlib.Path, required=True)
    args = parser.parse_args()
    try:
        build(args.compiler, args.license_file, args.notices, args.out)
    except (OSError, ValueError) as error:
        parser.exit(1, f"v0.14 archive failed: {error}\n")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
