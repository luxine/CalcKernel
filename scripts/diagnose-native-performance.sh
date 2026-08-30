#!/usr/bin/env bash
# Diagnostic evidence: never rewrites the frozen baseline or a gate result.
set -euo pipefail

diagnostic_repo="$(git rev-parse --show-toplevel)"
diagnostic_base="$diagnostic_repo/target/performance-v010-diagnostic"
diagnostic_out="$diagnostic_repo/target/performance-diagnostics"
mkdir -p "$diagnostic_out"
lscpu --json > "$diagnostic_out/cpu.json"
uname -a > "$diagnostic_out/host.txt"
rustc --version --verbose > "$diagnostic_out/rustc.txt"
test "$(git -C "$diagnostic_base" rev-parse HEAD)" = df816502876fba41676f9ebc190e4fadd18cd5a5
test -z "$(git -C "$diagnostic_base" status --porcelain)"
git -C "$diagnostic_base" rev-parse HEAD > "$diagnostic_out/v010-commit.txt"

apply_adapter() {
  local diagnostic_patch="$diagnostic_repo/benches/baselines/$1"
  test "$(sha256sum "$diagnostic_patch" | cut -d' ' -f1)" = "$2"
  git -C "$diagnostic_base" apply --check "$diagnostic_patch"
  git -C "$diagnostic_base" apply "$diagnostic_patch"
}
apply_adapter v0_10_linux_cpp_runtime_harness.patch 099305e8a9d5ff8d54e574b0fbd202a511f28a8543508f8c0ea06001704cdaff
apply_adapter v0_10_clang_cpu_harness.patch f22d58f4e2712e792a5b933376fe3a81fa1bd44a4cdb39b2790359ab5a40c7f1
apply_adapter v0_10_mir_optimizer_harness.patch 828138f376472b177d8bbd1aa4f7888ed323ec03d098e21a74abcfce32a98d0b
apply_adapter v0_10_proof_loop_harness.patch 316b64bf3e24ade271d870444bb66a85018c4dcb66229afce202da2d2b53af6e
cp "$diagnostic_repo/benches/fixtures/proof_loop.ck" "$diagnostic_base/benches/fixtures/proof_loop.ck"
cp "$diagnostic_repo/tests/fixtures/performance/native/proof_loop.ck" "$diagnostic_base/tests/fixtures/performance/native/proof_loop.ck"
printf 'proof\tbenches/fixtures/proof_loop.ck\n' >> "$diagnostic_base/benches/cases/native-cases.tsv"
git -C "$diagnostic_base" diff --check
git -C "$diagnostic_base" diff > "$diagnostic_out/v010-adapters.diff"
(
  cd "$diagnostic_base"
  cargo bench --features native-toolchain --bench ckc_perf -- --task check --cpu baseline
) 2>&1 | tee "$diagnostic_out/v010-benchmark.log"
cp "$diagnostic_base/target/ckc-perf/results.json" "$diagnostic_out/v010-results.json"
cp "$diagnostic_base/target/ckc-perf/v0-10-mir-optimizer.tsv" "$diagnostic_out/v010-optimizer.tsv"

# Preserve executable code for the failing scalar corpus independently of timing.
for diagnostic_version in v010 candidate; do
  if [[ "$diagnostic_version" == v010 ]]; then
    diagnostic_source="$diagnostic_base"
  else
    diagnostic_source="$diagnostic_repo"
  fi
  (
    cd "$diagnostic_source"
    cargo build --release --locked --features native-toolchain --bin ckc
    for diagnostic_mode in unchecked checked; do
      diagnostic_library="$diagnostic_out/$diagnostic_version-integer-$diagnostic_mode.so"
      target/release/ckc build tests/fixtures/performance/native/integer_accumulate.ck \
        --kind dynamic --out "$diagnostic_library" -O3 --cpu baseline \
        --overflow "$diagnostic_mode" --bounds "$diagnostic_mode"
      objcopy --dump-section ".text=$diagnostic_library.text" "$diagnostic_library"
      objdump -d "$diagnostic_library" > "$diagnostic_library.disassembly.txt"
    done
  )
done
sha256sum "$diagnostic_out"/*.text > "$diagnostic_out/code-sha256.txt"
