# CK 0.14 Design Blocker Rediagnosis 06

Review source: `design-adversarial-review-06.md`

Verdict: all three findings are confirmed blockers.

## R1. Input-map framing

Confirmed. The eight-byte magic and record field types do not determine the count
width or EOF rule. The revision will use the decision attachment's primitive
framing: `U32` count, concatenated records, `Text = U32 length || UTF-8`, fixed U64
and D32 fields, and no trailing byte.

## R2. CLI spelling

Confirmed. `--config` is the sole public product spelling already frozen in both
language documents. The schema-9 occurrence of `--workload` is an accidental
conflict, not a second product choice, and will become `--config`.

## R3. RSS provenance

Confirmed. Sampling cannot prove a peak. Because the two performance hosts are
already fixed to Linux, the revision will use the `wait4` result for the exact
direct compiler child, freeze Linux `ru_maxrss` KiB-to-byte conversion, and retain
one closed receipt. The ordinary build will use the identical supervisor and carry
its own receipt/digest, so neither side of the 2x comparison is asserted.

## Acceptance rule

All three corrections must be normative and mirrored where applicable, without
changing any threshold. A new ultra Sol review must return zero blockers before
planning begins.

## Applied revision and self-audit

- `CK_TUNE_INPUT_MAP` now uses `CKTIMAP1`, U32 big-endian count, the shared Text,
  U64, and D32 encodings, exact concatenation, a 0..64 bound, and exact EOF.
- The sole performance tuning recipe now uses the public `--config` spelling; no
  `--workload` occurrence remains in the v0.14 contract.
- Each tuned and ordinary resource observation now retains the same Linux
  direct-child receipt. Its command digest, `CLOCK_MONOTONIC_RAW` endpoints,
  successful `wait4` status, and kernel `ru_maxrss` KiB value deterministically
  derive wall time and peak bytes. Periodic samples are no longer authoritative.

Result: PASS for this repair round. English and Chinese summaries match, shared
schema fields are closed, and the 2x RSS threshold and every other gate remain
unchanged. Independent review is still required.
