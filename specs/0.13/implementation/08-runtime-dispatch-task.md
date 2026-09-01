# 阶段 08 任务：baseline-safe CPU detector、dispatcher 与 thunk

## 目标

实现 compiler-private runtime dispatch：baseline-safe capability detection、per-process normalized bitset、
per-root compiler-ranked variant selection、acquire/release exactly-once-compatible pointer publication和稳定
public thunk。失败/未知/矛盾/heterogeneous uncertainty 全部选 baseline，variant/support symbols 保持隐藏。

## 仓库落点

- 新建 `native/dispatch_runtime/` 的 private ABI/common 与 x86 CPUID/XGETBV、Linux AArch64 initial
  auxv HWCAP/HWCAP2、Darwin/Windows baseline adapter；修改 bootstrap/build manifest，身份独立。
- 修改 `src/backend/llvm/{kir_lower.rs,entry.rs,names.rs,verify.rs,fact_audit.rs}`，生成 public thunk、
  hidden implementation/table/support modules与 test-only detector seam。
- 修改 Native header/export/object/disassembly audit；static support symbol 以 target-set digest namespace。
- 新建 `tests/native/multiversion_dispatch.rs` 与 platform fixtures，扩展 libraries/executable/static/
  ABI/ownership/differential tests。

## TDD 顺序

1. 写 detector table RED：x86 complete v3/v4 bits + OSXSAVE/XCR0 YMM/opmask/ZMM；Linux AArch64
   initial auxv SVE/SVE2 usable state；unknown future/contradictory/query failure 全部 baseline。
2. 写 baseline containment RED：detector、resolver、thunk 与 baseline module 不含任何 optional ISA；
   each enhanced module 不超声明 feature，跨 variant reference/LTO 泄漏拒绝。
3. 写 dispatch RED：每 root 依 compiler-ranked order 选择最高兼容且实际有收益的 variant，而非数字
   tier；baseline-only 与 no-compatible tier 稳定；production 无 env/public force override。
4. 写 concurrency RED：首调用可并行计算兼容答案，仅发布 verified pointer；capability state process-local
   恰初始化一次，后续每 call 一个 atomic load + indirect tail call，不再 CPUID/HWCAP。
5. 写 ABI RED：public symbol/address 始终是 thunk；calling convention、slice flatten、checked status/
   result slot、alignment/unwind/visibility/header/export 与单版本完全一致；hidden digest symbols 不可查找。
6. 写 failure/test seam RED：private seam 可强制“兼容”variant做 differential，但不能强制 unsupported
   feature production execution；malformed table/pointer/digest withholding artifact。
7. 写 real-hardware/differential RED：支持的本机 tier 真选择，不支持主机 baseline；ordinary/forced
   compatible variants/production dispatch 在 training/held-out/adversarial input 语义一致。
8. 实现 freestanding detector/runtime、generated table/thunk与 independent symbol/feature verifier；
   静态、动态、可执行链接单元先作为 named objects 输出，最终统一装配留给阶段 09。

## 实现边界

- 不解析 mutable text，不联网、不用 LLVM runtime，不增加 CK-visible CPU API。
- initial auxiliary vector 在进程初始化的 private support 中捕获；查询不可用即 baseline。
- dynamic loader/symbol resolution 与 first-call resolution 性能分开记录；steady-state 不允许重复解析。

## RED/GREEN 证据

记录 detector mutation、并发次数/public address、real hardware capability、baseline/variant disassembly、
ABI/differential digest 到 `target/acceptance/v0.13/stage-08/`。
