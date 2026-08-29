#!/usr/bin/env bash
set -euo pipefail

if [[ $# -ne 1 || ! -f "$1" || ! -x "$1" ]]; then
  echo "usage: $0 <ckc-release-executable>" >&2
  exit 2
fi

readonly ckc_script_root="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd -P)"
readonly ckc_candidate="$(cd "$(dirname "$1")" && pwd -P)/$(basename "$1")"
readonly ckc_audit_root="$(mktemp -d "${TMPDIR:-/tmp}/ckc-jit-audit.XXXXXX")"
trap 'rm -rf -- "$ckc_audit_root"' EXIT

ckc_run_candidate="$ckc_candidate"
if [[ "$(uname -s)" == Darwin ]]; then
  readonly ckc_entitlements="$ckc_script_root/../native/macos/ckc-jit.entitlements.plist"
  [[ -f "$ckc_entitlements" ]] || {
    echo "JIT memory audit: missing entitlement policy" >&2
    exit 1
  }
  ckc_run_candidate="$ckc_audit_root/ckc"
  cp "$ckc_candidate" "$ckc_run_candidate"
  chmod 755 "$ckc_run_candidate"
  codesign --force --sign - --options runtime \
    --entitlements "$ckc_entitlements" "$ckc_run_candidate"
  codesign --verify --strict --verbose=2 "$ckc_run_candidate"

  readonly ckc_signature="$(codesign -dvv "$ckc_run_candidate" 2>&1)"
  grep -Eq 'flags=.*runtime' <<<"$ckc_signature" || {
    echo "JIT memory audit: candidate is not hardened" >&2
    exit 1
  }
  codesign -d --entitlements :- "$ckc_run_candidate" \
    >"$ckc_audit_root/entitlements.plist" 2>/dev/null
  readonly ckc_entitlement_dump="$(plutil -p "$ckc_audit_root/entitlements.plist")"
  [[ "$(grep -c '=>' <<<"$ckc_entitlement_dump")" -eq 1 ]] &&
    grep -q '"com.apple.security.cs.allow-jit" => true' \
      <<<"$ckc_entitlement_dump" || {
      echo "JIT memory audit: hardened candidate has unexpected entitlements" >&2
      exit 1
    }
fi

printf '%s\n' \
  'fn main() -> i32 {' \
  '    print_i32(42);' \
  '    print_newline();' \
  '    return 0;' \
  '}' >"$ckc_audit_root/program.ck"

set +e
PATH='' CKC_INTERNAL_JIT_AUDIT=1 "$ckc_run_candidate" run \
  "$ckc_audit_root/program.ck" --no-cache \
  >"$ckc_audit_root/stdout" 2>"$ckc_audit_root/stderr"
readonly ckc_status=$?
set -e

[[ $ckc_status -eq 0 ]] || {
  sed 's/^/JIT memory audit: /' "$ckc_audit_root/stderr" >&2
  exit 1
}
[[ "$(cat "$ckc_audit_root/stdout")" == 42 ]] || {
  echo "JIT memory audit: program stdout mismatch" >&2
  exit 1
}
readonly ckc_report="$(cat "$ckc_audit_root/stderr")"
[[ "$(grep -c '^CKC_JIT_AUDIT_V1 ' <<<"$ckc_report")" -eq 1 ]] &&
  [[ "$(wc -l <"$ckc_audit_root/stderr" | tr -d ' ')" -eq 1 ]] || {
    echo "JIT memory audit: expected exactly one audit record" >&2
    exit 1
  }
for ckc_field in \
  ' allocations=[1-9][0-9]*' \
  ' relocation=rw-nx' \
  ' code=rx' \
  ' data=nx' \
  ' icache=flushed' \
  ' icache-count=[1-9][0-9]*'; do
  grep -Eq "$ckc_field" <<<"$ckc_report" || {
    echo "JIT memory audit: missing policy evidence $ckc_field" >&2
    exit 1
  }
done

case "$(uname -s)" in
  Darwin)
    grep -q ' layer=jitlink' <<<"$ckc_report"
    if grep -q ' map-jit=yes thread-wx-supported=yes thread-wx=yes' \
      <<<"$ckc_report"; then
      :
    elif grep -q ' map-jit=no thread-wx-supported=no thread-wx=no' \
      <<<"$ckc_report"; then
      :
    else
      echo "JIT memory audit: inconsistent Darwin W^X capability tuple" >&2
      exit 1
    fi
    ;;
  Linux)
    grep -q ' layer=jitlink' <<<"$ckc_report"
    grep -q ' map-jit=no' <<<"$ckc_report"
    grep -q ' thread-wx-supported=no' <<<"$ckc_report"
    grep -q ' thread-wx=no' <<<"$ckc_report"
    ;;
  *)
    echo "JIT memory audit: unsupported Unix host" >&2
    exit 2
    ;;
esac

echo "JIT memory audit passed: $ckc_candidate"
