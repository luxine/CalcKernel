#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 || ! -d "$1" ]]; then
  echo "usage: $0 <native-acceptance-directory>" >&2
  exit 2
fi

readonly ckc_audit_root="$(cd "$1" && pwd -P)"
readonly ckc_runtime_root="$ckc_audit_root/runtime"
readonly ckc_forbidden_dependency='LLVM|LLD|Clang|CalcKernel|libck|libstdc\+\+|libc\+\+'
readonly ckc_forbidden_symbol='(^|_)(malloc|calloc|realloc|free|printf|fprintf|sprintf|snprintf|vsnprintf|setlocale|localeconv|__stack_chk_fail)(@|$)'

require_file() {
  [[ -f "$1" ]] || {
    echo "native artifact audit: missing $1" >&2
    exit 1
  }
}

audit_runtime_objects() {
  require_file "$ckc_runtime_root/SHA256SUMS"
  if command -v sha256sum >/dev/null 2>&1; then
    (cd "$ckc_runtime_root" && sha256sum --check --strict SHA256SUMS)
  else
    (cd "$ckc_runtime_root" && shasum -a 256 --check SHA256SUMS)
  fi
  local ckc_object
  for ckc_object in "$ckc_runtime_root"/*.{o,obj}; do
    [[ -e "$ckc_object" ]] || continue
    if nm -u "$ckc_object" 2>/dev/null | awk '{print $NF}' | grep -Ei "$ckc_forbidden_symbol" >/dev/null; then
      echo "native artifact audit: forbidden runtime symbol in $ckc_object" >&2
      exit 1
    fi
  done
}

audit_macho() {
  local ckc_object="$ckc_audit_root/module.o"
  local ckc_archive="$ckc_audit_root/libmodule.a"
  local ckc_dynamic="$ckc_audit_root/libmodule.dylib"
  local ckc_executable="$ckc_audit_root/program"
  require_file "$ckc_object"
  require_file "$ckc_archive"
  require_file "$ckc_dynamic"
  require_file "$ckc_executable"
  file "$ckc_object" | grep 'Mach-O.*object' >/dev/null
  file "$ckc_archive" | grep 'current ar archive' >/dev/null
  file "$ckc_dynamic" | grep 'Mach-O.*dynamically linked shared library' >/dev/null
  file "$ckc_executable" | grep 'Mach-O.*executable' >/dev/null
  [[ "$(nm -gU "$ckc_dynamic" | awk '{print $NF}')" == "_answer" ]] || {
    echo "native artifact audit: unexpected Mach-O exports" >&2
    exit 1
  }
  if otool -L "$ckc_dynamic" "$ckc_executable" | awk '/^\t/{print $1}' | grep -Ei "$ckc_forbidden_dependency" >/dev/null; then
    echo "native artifact audit: forbidden Mach-O dependency" >&2
    exit 1
  fi
  local ckc_dependency
  while IFS= read -r ckc_dependency; do
    [[ -z "$ckc_dependency" || "$ckc_dependency" == /usr/lib/libSystem.B.dylib ]] || {
      echo "native artifact audit: unexpected executable dependency $ckc_dependency" >&2
      exit 1
    }
  done < <(otool -L "$ckc_executable" | tail -n +2 | awk '{print $1}')
  codesign --verify --verbose=2 "$ckc_executable"
  codesign -dvv "$ckc_executable" 2>&1 | grep 'Signature=adhoc' >/dev/null
  otool -l "$ckc_executable" | grep -A6 'LC_BUILD_VERSION' | grep 'minos 11.0' >/dev/null
  local ckc_undefined
  while IFS= read -r ckc_undefined; do
    case "$ckc_undefined" in
      ""|__exit|_fcntl|_signal|_write|dyld_stub_binder) ;;
      *)
        echo "native artifact audit: unexpected executable import $ckc_undefined" >&2
        exit 1
        ;;
    esac
  done < <(nm -u "$ckc_executable" | awk '{print $NF}')
}

audit_elf() {
  local ckc_object="$ckc_audit_root/module.o"
  local ckc_archive="$ckc_audit_root/libmodule.a"
  local ckc_dynamic="$ckc_audit_root/libmodule.so"
  local ckc_executable="$ckc_audit_root/program"
  require_file "$ckc_object"
  require_file "$ckc_archive"
  require_file "$ckc_dynamic"
  require_file "$ckc_executable"
  readelf -h "$ckc_object" | grep 'Type:.*REL' >/dev/null
  file "$ckc_archive" | grep 'current ar archive' >/dev/null
  readelf -h "$ckc_dynamic" | grep 'Type:.*DYN' >/dev/null
  readelf -h "$ckc_executable" | grep 'Type:.*EXEC' >/dev/null
  if readelf -d "$ckc_dynamic" "$ckc_executable" | grep '(NEEDED)' >/dev/null; then
    echo "native artifact audit: ELF artifact has a dynamic dependency" >&2
    exit 1
  fi
  [[ "$(nm -D --defined-only "$ckc_dynamic" | awk '{print $NF}')" == "answer" ]] || {
    echo "native artifact audit: unexpected ELF exports" >&2
    exit 1
  }
  if nm -u "$ckc_executable" | grep . >/dev/null; then
    echo "native artifact audit: static ELF executable has undefined symbols" >&2
    exit 1
  fi
  if readelf -p .comment "$ckc_object" "$ckc_dynamic" "$ckc_executable" 2>/dev/null | grep -Ei "$ckc_forbidden_dependency" >/dev/null; then
    echo "native artifact audit: forbidden ELF producer marker" >&2
    exit 1
  fi
}

audit_runtime_objects
case "$(uname -s)" in
  Darwin) audit_macho ;;
  Linux) audit_elf ;;
  *) echo "native artifact audit: unsupported Unix host" >&2; exit 2 ;;
esac

echo "native artifact audit passed: $ckc_audit_root"
