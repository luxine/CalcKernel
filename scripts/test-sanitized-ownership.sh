#!/usr/bin/env bash
set -euo pipefail

sanitizer_cxxflags='-fsanitize=address,undefined -fno-omit-frame-pointer'
sanitizer_rustflags='-C link-arg=-fsanitize=address -C link-arg=-fsanitize=undefined'

if [[ "$(uname -s)" == Linux ]]; then
  CXXFLAGS="${sanitizer_cxxflags}" \
  RUSTFLAGS="${sanitizer_rustflags}" \
  ASAN_OPTIONS='detect_leaks=1:halt_on_error=1' \
  UBSAN_OPTIONS='halt_on_error=1:print_stacktrace=1' \
    cargo test --all-features --locked --test native ownership --target-dir target/sanitized
  exit 0
fi

if [[ "$(uname -s)" == Darwin ]]; then
  echo "sanitized ownership is a Linux-only gate: this Apple ASan runtime cannot initialize reliably" >&2
  exit 0
fi

echo "sanitized ownership is unsupported on $(uname -s)" >&2
exit 1
