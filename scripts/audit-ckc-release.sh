#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 || ! -f "$1" || ! -x "$1" ]]; then
  echo "usage: $0 <ckc-release-executable>" >&2
  exit 2
fi

readonly ckc_candidate="$(cd "$(dirname "$1")" && pwd -P)/$(basename "$1")"
readonly ckc_forbidden='LLVM|LLD|Clang|CalcKernel|libck'

case "$(uname -s)" in
  Darwin)
    readonly ckc_dependencies="$(otool -L "$ckc_candidate" | tail -n +2 | awk '{print $1}')"
    if grep -Eiq "$ckc_forbidden" <<<"$ckc_dependencies"; then
      echo "ckc release audit: compiler implementation dependency detected" >&2
      exit 1
    fi
    while IFS= read -r ckc_dependency; do
      case "$ckc_dependency" in
        ""|/usr/lib/*|/System/Library/*) ;;
        *)
          echo "ckc release audit: non-system Darwin dependency $ckc_dependency" >&2
          exit 1
          ;;
      esac
    done <<<"$ckc_dependencies"
    if otool -l "$ckc_candidate" | grep -Eq 'LC_RPATH|LC_LOAD_DYLIB.*@(rpath|loader_path|executable_path)'; then
      echo "ckc release audit: relocatable runtime search path detected" >&2
      exit 1
    fi
    codesign --verify --strict --verbose=2 "$ckc_candidate"
    ;;
  Linux)
    readonly ckc_dynamic="$(readelf -d "$ckc_candidate")"
    if grep -Eiq "$ckc_forbidden|libstdc\+\+|libc\+\+" <<<"$ckc_dynamic"; then
      echo "ckc release audit: dynamic compiler or C++ runtime dependency detected" >&2
      exit 1
    fi
    if grep -Eq '\((RPATH|RUNPATH)\)' <<<"$ckc_dynamic"; then
      echo "ckc release audit: runtime search path detected" >&2
      exit 1
    fi
    ;;
  *)
    echo "ckc release audit: unsupported Unix host" >&2
    exit 2
    ;;
esac

readonly ckc_verbose="$($ckc_candidate --version --verbose)"
readonly ckc_licenses="$($ckc_candidate licenses)"
grep -q '^LLVM: 22\.1\.8$' <<<"$ckc_verbose"
grep -q '^===== LLVM Project 22\.1\.8' <<<"$ckc_licenses"
echo "ckc release audit passed: $ckc_candidate"
