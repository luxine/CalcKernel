# CalcKernel V0.9 Compatibility Policy

[简体中文](../zh-CN/project/compatibility.md)

This document is the normative compatibility authority for release line
`0.9.x`.

Patch releases in `0.9.x` preserve backward compatibility for:

- every CK source program accepted by 0.9.0 and its observable semantics;
- stable diagnostic identifiers and their triggering category;
- command names, accepted flags/aliases/defaults, argument precedence,
  stdout/stderr class, exit success/failure, and artifact naming;
- textual MIR syntax, deterministic printing, and instruction meaning;
- documented C, WASM, LLVM, checked-mode, slice, void, and exported function ABI;
- the six native release target/archive names and checksum sidecars.

Patch releases may fix rejection of invalid input, improve diagnostic prose and
caret precision, add non-breaking documentation, optimize without observable
semantic change, and add APIs or flags whose defaults preserve prior behavior.

Internal Rust module/file paths, private Rust items, test organization,
implementation algorithms, benchmark measurements, build-cache contents, and
undocumented backend internals are not compatibility promises. The public Rust
re-exports intentionally used by repository tests remain stable for `0.9.x`.

`0.10.0` may make a breaking language, diagnostic, CLI, MIR, or ABI change only
when the change is explicitly documented and accompanied by migration guidance.
A future `1.0.0` begins the long-term compatibility commitment; V0.9 does not
claim 1.0 stability.
