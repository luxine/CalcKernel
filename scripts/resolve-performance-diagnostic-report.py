#!/usr/bin/env python3
"""Resolve retained historical diagnostic evidence without rerunning a gate."""

import argparse
import hashlib
import json
from pathlib import Path
import re
import sys


def require_regular_file(path: Path, label: str) -> None:
    if path.is_symlink() or not path.is_file() or path.stat().st_size <= 0:
        raise ValueError(f"{label} must be a nonempty regular file")


def resolve_cumulative_report(report_path: Path) -> Path:
    require_regular_file(report_path, "historical schema-8 report")
    if report_path.parent.is_symlink() or not report_path.parent.is_dir():
        raise ValueError("historical schema-8 directory must be a real directory")
    report = json.loads(report_path.read_text(encoding="utf-8"))
    if (not isinstance(report, dict) or type(report.get("schemaVersion")) is not int
            or report["schemaVersion"] != 8 or report.get("candidateVersion") != "0.13.0"):
        raise ValueError("historical diagnostic requires schema 8 for candidate 0.13.0")
    directory = report.get("evidenceDirectory")
    if (not isinstance(directory, str)
            or re.fullmatch(r"v013-measurement-[0-9]+-[0-9]+", directory) is None):
        raise ValueError("unsafe historical evidence directory")
    evidence = report_path.parent / directory
    if evidence.is_symlink() or not evidence.is_dir():
        raise ValueError("historical evidence directory must be a real directory")
    record = report.get("cumulativeSchemaSeven")
    if (not isinstance(record, dict) or set(record) != {"file", "bytes", "sha256"}
            or record["file"] != "results-schema7.json"
            or type(record["bytes"]) is not int or record["bytes"] <= 0
            or not isinstance(record["sha256"], str)
            or re.fullmatch(r"[0-9a-f]{64}", record["sha256"]) is None):
        raise ValueError("invalid cumulative schema-7 record")
    cumulative = evidence / record["file"]
    require_regular_file(cumulative, "cumulative schema-7 report")
    with cumulative.open("rb") as source:
        digest = hashlib.file_digest(source, "sha256").hexdigest()
    if cumulative.stat().st_size != record["bytes"] or digest != record["sha256"]:
        raise ValueError("cumulative schema-7 identity mismatch")
    return cumulative


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("report", type=Path)
    args = parser.parse_args()
    try:
        print(resolve_cumulative_report(args.report))
    except (OSError, ValueError) as error:
        print(f"performance diagnostic report resolution failed: {error}", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    sys.exit(main())
