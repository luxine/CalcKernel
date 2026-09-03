# Implementation blocker 11: the pinned V0.13 replay aborted before measurement

Run `33757231836` established a blocker rather than a performance observation:
the exact V0.13 replay compiler aborted in LLVM while assigning a name to a
`void` call.  The failure was reproduced against the named fixture and traced to
the bridge call builder.  V0.13 now omits names for void results and carries a
subprocess regression; V0.14 already had the equivalent bridge behavior.

The only valid V0.14 repair is to replace the pre-fix replay identity with the
complete repaired V0.13 candidate, then regenerate its schema-8 evidence under
the original checker.  The pin and manifest digest are updated consistently as
specified by `implementation-design-correction-12.md`.  No threshold, corpus,
sample statistic, or required job is removed.  This blocker remains open until
the replacement exact-SHA remote replay and schema-9 job pass.
