# Native Benchmark Summary Schema

`cargo bench --bench ckc_perf` writes:

- `build/perf/latest.summary.json`
- `build/perf/latest.summary.md`

The JSON file is intentionally simple so it can be checked into a local
baseline or compared by external tooling without depending on a JavaScript
runner.

Top-level fields:

- `schemaVersion`: currently `1`.
- `command`: the native Cargo benchmark command.
- `generatedAtUnixSeconds`: local generation timestamp.
- `target`: operating system and CPU architecture reported by Rust.
- `iterations`: measured iterations per case/task pair.
- `warmup`: unmeasured warmup iterations per case/task pair.
- `results`: measured case/task rows.

Each result row records the CK source case, compiler task, stage, sample counts,
duration statistics in nanoseconds, and `outputUnits` for the generated output
size or checked symbol count.
