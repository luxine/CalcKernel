#!/usr/bin/env bash
# Failure-only evidence collection; this never replaces the required native suite.
set -euo pipefail

if [[ $(uname -s) != Darwin ]]; then
  echo 'Darwin native diagnostics require macOS' >&2
  exit 1
fi

diagnostic_dir=${1:-target/native-diagnostics}
mkdir -p "$diagnostic_dir"
cargo test --all-features --locked --test native --no-run --message-format=json \
  | tee "$diagnostic_dir/test-artifacts.jsonl"
native_test=$(jq -sr '
  [.[] | select(.reason == "compiler-artifact" and .target.name == "native"
                and .profile.test and .executable != null) | .executable] | unique
  | if length == 1 then .[0] else error("expected one native test executable") end
' "$diagnostic_dir/test-artifacts.jsonl")

replay() {
  local mode=$1
  shift
  if RUST_BACKTRACE=1 xcrun lldb --batch \
    --one-line 'settings set target.disable-aslr false' \
    --one-line run \
    --one-line-on-crash 'thread backtrace all' \
    -- "$native_test" "$@" 2>&1 | tee "$diagnostic_dir/lldb-$mode.log"; then
    echo "$mode debugger exit: 0" | tee -a "$diagnostic_dir/status.txt"
  else
    echo "$mode debugger exit: $?" | tee -a "$diagnostic_dir/status.txt"
  fi
}

# Serial replay identifies the active test; parallel replay retains scheduling
# conditions if the failure disappears when tests are serialized.
replay serial --test-threads=1 --nocapture
replay parallel --nocapture

for report_dir in "$HOME/Library/Logs/DiagnosticReports" /Library/Logs/DiagnosticReports; do
  if [[ -d "$report_dir" ]]; then
    find "$report_dir" -maxdepth 1 -type f \
      \( -name 'native-*.ips' -o -name 'native-*.crash' \
         -o -name 'program-*.ips' -o -name 'program-*.crash' \
         -o -name 'ckc-*.ips' -o -name 'ckc-*.crash' \) \
      -exec cp {} "$diagnostic_dir/" \;
  fi
done
