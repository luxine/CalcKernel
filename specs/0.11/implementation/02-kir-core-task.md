# 阶段 02 任务：KIR 核心、SSA 与 mode-specific guard

## 目标

从裁剪后的稳定 MIR 构造确定的 KIR：标量 SSA、显式控制流、初始 region Memory SSA、
有序 runtime/may-fail operation 和由安全模式决定的 guard。O0 只构造并验证。

## 仓库落点

- 新建 `src/ir/kir/{mod.rs,model.rs,builder.rs,dominance.rs,print.rs,validate.rs}`，由
  `src/ir/mod.rs` 导出有意公开的 inspection API。
- `src/ir/reachability.rs`：统一 consumer roots/capability check，增加 inspection
  consumer；修复现有 WAT/WASM/run/build 路径差异的共享入口。
- `tests/ir/kir.rs` 并登记到 `tests/ir.rs`。

## 核心模型

- 强类型 ID newtypes：function/block/value/instruction/region/memory-version/fact/proof。
- `KirBuildConfig` 固定 consumer、overflow、bounds、sanitizer；作为 module identity 的一
  部分打印。
- block parameters/phi 只引用支配定义；使用确定的 sealed-block SSA 或等价算法处理
  MIR mutable locals、循环和 join，不依赖 HashMap iteration。
- pointer/slice origin 建立 stable region；sub-slice 保存 parent region 与 symbolic byte
  interval。阶段 04 再细化 alias partitions。
- checked arithmetic 产生 result/overflow condition，随后显式 ordered guard；division/
  modulo zero、slice index、subslice start/end checks 保留当前首错顺序。
- unchecked integer operation 标记 modular；f64 始终 strict。
- runtime print、may-fail call 和 return/status 是显式 ordered effects。

## TDD 顺序

1. 写 model/printer determinism red tests，再实现 ID、model 与 printer。
2. 写 straight-line SSA red tests；实现 params/locals/temps 的单定义转换。
3. 写 if/while/break/continue/short-circuit phi 与 dominance red tests；实现 CFG、dominator、
   sealed SSA，并用 mutation 证明 use-before-def/错误 phi 被拒绝。
4. 写 consumer reachability red tests，覆盖 C/WASM export roots、native main root、inspection
   union、不可达 print、可达 unsupported runtime；统一先裁剪后构造。
5. 写 O0 checked/unchecked guard ordering red tests，逐个迁移 C/LLVM 现有 check-site 语义到
   builder；验证 KIR 不依赖 backend 才补 guard。
6. 写 deterministic print red tests：重复 50 次及不同 map 插入顺序 byte-identical，且无
   path/address/time。

## 明确不做

不删除任何 guard，不执行可选优化，不完成 alias/effect analysis，不切换正式 backend。
旧 MIR optimizer 暂作为 shadow baseline，最终阶段删除正式调用路径。
