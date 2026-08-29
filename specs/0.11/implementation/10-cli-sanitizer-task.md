# 阶段 10 任务：CLI inspection 与 contract sanitizer

## 目标

接通统一 KIR 编译入口、确定的 inspection surfaces、unsafe header 注释，以及只用于
`run`/executable build 的精确 contract sanitizer。

## 仓库落点

- `src/cli/args.rs`/`mod.rs`/`commands.rs`：`emit-kir`、`--print-facts`、
  `--print-effect-summaries`、`--explain-optimization`、`--sanitize-contracts`。
- 提取一个 `compile_kir` helper，所有非 `emit-mir` artifact command 使用同一
  consumer/mode pipeline；删除命令间 reachability/mode 顺序差异。
- `src/cli/cache/*`：cache key/manifest 纳入 KIR contract version、sanitizer 与全部
  object-affecting inspection-independent options。
- `src/backend/header.rs`：exported unsafe contract 的 normalized flattened slice comment。
- `native/runtime/common/contract.c` 与 header/provenance：zero-dependency exact signed-limb
  affine evaluator、checked address interval 和 CKR0007；Runtime ABI 从 1 升到 2。
- `tests/cli/kir_inspection.rs`、`tests/native/contract_sanitizer.rs`。

## TDD 顺序

1. 写 args red tests：新 command/flags 的 allowed matrix、default mode/O-level、unsupported
   WASM mode、sanitizer 仅 run 与 `build --kind executable`，错误时不写文件。
2. 写 emit-kir/facts/effects/explain determinism red tests；实现排序、retained reason、
   TrustedContract 标记，禁止 path/address/time/unordered iteration。
3. 写 header red tests：affine/noalias/aligned/effects 规范化，`x.data/x.len` 映射为 C ABI
   的 `x_data/x_len`，注释不改变 declaration bytes/shape。
4. 写 normal-mode red tests，证明 O0–O3 不插入 contract check。
5. 写 sanitizer positive/negative red tests：内部 unsafe call、exported unsafe entry、每个
   predicate、多个 requires、递归、O0–O3；精确 stderr LF 与 exit 246。
6. 写数学极值 red tests：u64/i64 extremes、大 coefficient/term count、negative affine、
   element-byte-length overflow、address-end overflow、zero-length noalias。实现 fixed-size
   exact limbs 或等价算法，不能用 wrapping/host pointer comparison。
7. 写 cache red tests：sanitizer 与 normal object 不碰撞，failure/corruption 不提交；run
   public-parent/child 仍保留 stdin/stdout/stderr/status 契约。

## 实现判定

- sanitizer checks 是不可消除的 ordered contract guards；正常模式完全没有这些 guards。
- effect ceiling 仍只在编译期检查，sanitizer 不伪造动态 write tracking。
- CKR0007/246 只属于 sanitizer violation；既有 CKR0001–0006 bytes/status 不漂移。
- exact evaluator 与 runtime 均不引入动态库或外部工具依赖。

## 明确不做

不把 sanitizer 暴露为 library ABI，不把其结果当 benchmark evidence，不新增通用 bigint
语言类型。
