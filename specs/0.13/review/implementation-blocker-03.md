# Implementation blocker 03: named void call aborted assertion-enabled LLVM

The V0.14 x86-64 performance job in run `33757231836` replayed the then-pinned
V0.13 compiler and aborted while building
`benches/fixtures/pgo/call_constant_length.ck` as a multiversion dynamic
library.  LLVM's assertion-enabled build rejected an attempt to assign an SSA
name to a call whose result type is `void`.

The bridge now creates every call without a name and applies the requested name
only when the callee return type is non-void.  A subprocess regression builds
the exact fixture with `-O3 --cpu multiversion --overflow unchecked --bounds
unchecked`, so an LLVM abort is isolated and reported as a failed test rather
than terminating the test harness without context.

The same repair also carries forward V0.12's target-capability guard for dead
loop-dependence analysis; neither performance threshold nor historical result
is changed.  The complete non-Native suite, all 159 Native tests, format check,
and Clippy with warnings denied pass locally.  Exact-SHA remote jobs and the
resulting replay bundle remain required for acceptance.
