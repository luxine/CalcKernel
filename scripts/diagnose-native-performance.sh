#!/usr/bin/env bash
# Diagnostic evidence: inspect the measured files, never rebuild or retime a kernel.
set -euo pipefail

diagnostic_repo="$(git rev-parse --show-toplevel)"
diagnostic_bundle="${CKC_V010_RUNTIME_BUNDLE:?prepare the pinned replay bundle first}"
diagnostic_out="$diagnostic_repo/target/performance-diagnostics"
mkdir -p "$diagnostic_out"
if command -v lscpu >/dev/null; then
  lscpu --json > "$diagnostic_out/cpu.json"
fi
uname -a > "$diagnostic_out/host.txt"
rustc --version --verbose > "$diagnostic_out/rustc.txt"
git rev-parse HEAD > "$diagnostic_out/candidate-commit.txt"
test -s "$diagnostic_bundle/preparation.log"
test -s "$diagnostic_bundle/replay.tsv"

# Resolve only fixed report basenames. The report and bundle themselves are uploaded.
python3 -B - "$diagnostic_repo/target/ckc-perf/results.json" "$diagnostic_bundle" \
  > "$diagnostic_out/measured-libraries.tsv" <<'PY'
import json, pathlib, re, sys
report_path = pathlib.Path(sys.argv[1])
report = json.loads(report_path.read_text())
directory = report["evidenceDirectory"]
if not isinstance(directory, str) or re.fullmatch(r"measurement-[0-9]+-[0-9]+", directory) is None:
    raise ValueError("unsafe evidence directory")
for baseline, entries in [(False, report["measuredArtifacts"]), (True, report["runtimeReplay"]["artifacts"])]:
    root = pathlib.Path(sys.argv[2]) if baseline else report_path.parent / directory
    for entry in entries:
        case, mode = entry["case"], entry["mode"]
        channel = "replayNative" if baseline else entry["channel"]
        if case not in {"branch_mix", "integer_accumulate", "proof_loop", "remainder_chain"}:
            raise ValueError("unknown diagnostic case")
        if mode not in {"checked", "unchecked"}:
            raise ValueError("unknown diagnostic mode")
        suffixes = {"candidateNative": "-native", "currentClang": "-clang", "replayClang": "-replay-clang", "replayNative": ""}
        suffix = suffixes[channel]
        file = entry["file"]
        if file not in {f"{case}-{mode}{suffix}{ext}" for ext in [".so", ".dylib", ".dll"]}:
            raise ValueError("unsafe diagnostic artifact basename")
        if re.fullmatch(r"[0-9a-f]{64}", entry["sha256"]) is None:
            raise ValueError("invalid expected artifact digest")
        print(f"{case}-{mode}-{channel}\t{root / file}\t{entry['sha256']}")
PY
test -s "$diagnostic_out/measured-libraries.tsv"
if command -v sha256sum >/dev/null; then
  diagnostic_hash=(sha256sum)
else
  diagnostic_hash=(shasum -a 256)
fi
diagnostic_objdump=("${CKC_LLVM_PREFIX:?}/bin/llvm-objdump")
if [[ ! -x "${diagnostic_objdump[0]}" ]]; then
  diagnostic_objdump=(objdump)
fi
while IFS=$'\t' read -r diagnostic_label diagnostic_library diagnostic_expected; do
  test -s "$diagnostic_library"
  test ! -L "$diagnostic_library"
  diagnostic_actual="$("${diagnostic_hash[@]}" "$diagnostic_library" | cut -d' ' -f1)"
  printf '%s  %s\n' "$diagnostic_actual" "$diagnostic_library" >> "$diagnostic_out/whole-library-sha256.txt"
  test "$diagnostic_actual" = "$diagnostic_expected"
  # Disassemble all executable sections, including LLVM large-model .ltext.
  # Whole-library hashes are primary evidence; no section-only equality inference.
  "${diagnostic_objdump[@]}" -d "$diagnostic_library" > "$diagnostic_out/$diagnostic_label.disassembly.txt"
done < "$diagnostic_out/measured-libraries.tsv"
