# 阶段 12 任务：Profile Runtime 原子与持久发布可移植性

> **For agentic workers:** REQUIRED SUB-SKILL: Use `superpowers:executing-plans`;
> execute inline without subagents.

## 目标

修复六个 Native host 已暴露的 profile-generation 根因：MSVC 不能使用
`<stdatomic.h>`、Linux AArch64 持久 shard 发布失败，以及 Darwin x86-64
引入非冻结 `_fstat$INODE64` 依赖。不得改变 Runtime ABI 2、CKPROF01、
CKPART01 或公开状态码。

## 仓库落点与冻结接口

- 新增 `native/profile_runtime/include/ckc_profile_atomic.h`，只暴露内部
  `CkProfileAtomicU32/U64` 与 load/store/CAS 操作。MSVC 使用
  `InterlockedCompareExchange`、`InterlockedExchange`、
  `InterlockedCompareExchange64`；Unix 使用 C11 lock-free atomics。
- 修改 `native/profile_runtime/common/collector.c`，删除直接 `_Atomic` 和
  `atomic_*` 调用，全部路由到内部 wrapper；MSVC 的 full barrier 允许强于
  acquire/release，但不能弱于原语义。
- 修改 `native/profile_runtime/platform/linux.c` 与 `platform/darwin.c`，让
  open-directory、same-handle identity、create-new temp、完整写入、file sync、
  no-replace rename、directory sync、失败清理和 close 在所有已声明架构上使用
  正确 ABI。Linux raw syscall stat buffer 用架构显式布局和编译期 offset/size
  断言；Darwin 不再产生未声明的 `$INODE64` 符号。
- 修改 `native/profile_runtime/include/ckc_profile_platform.h`，增加仅 runtime
  内部可见的 open/identity/create/write/file-sync/rename/directory-sync failure
  枚举；`collector.c` 仍将其稳定映射到公开 43/44/45 状态。
- 更新 `native/profile_runtime/provenance.toml` 的 runtime file 列表，确保新
  header 进入 build-time component digest。
- 扩展 `tests/contracts/native_toolchain.rs`、`tests/native/profile_generation.rs`
  与 `tests/native/artifacts.rs`；不增加新的 integration-test driver。

内部原子 API 固定为：

```c
typedef struct { /* platform-owned storage */ } CkProfileAtomicU32;
typedef struct { /* platform-owned storage */ } CkProfileAtomicU64;
static uint32_t ck_profile_atomic_u32_load_acquire(const CkProfileAtomicU32 *);
static uint32_t ck_profile_atomic_u32_load_relaxed(const CkProfileAtomicU32 *);
static void ck_profile_atomic_u32_store_release(CkProfileAtomicU32 *, uint32_t);
static void ck_profile_atomic_u32_store_relaxed(CkProfileAtomicU32 *, uint32_t);
static int ck_profile_atomic_u32_compare_exchange_acq_rel(
    CkProfileAtomicU32 *, uint32_t *expected, uint32_t desired);
static uint64_t ck_profile_atomic_u64_load_relaxed(const CkProfileAtomicU64 *);
static int ck_profile_atomic_u64_compare_exchange_relaxed(
    CkProfileAtomicU64 *, uint64_t *expected, uint64_t desired);
```

## TDD 顺序

1. 在 `tests/contracts/native_toolchain.rs` 添加 RED
   `profile_runtime_atomic_abstraction_should_compile_for_c11_and_msvc`：断言
   collector 不含 `<stdatomic.h>`/`_Atomic`，新 header 的 Unix 分支包含
   `ATOMIC_LLONG_LOCK_FREE == 2`，MSVC 分支包含三种 Interlocked primitive，
   provenance 精确列出 header。运行该单测并确认因 header 缺失失败。
2. 添加 RED `profile_runtime_platforms_should_name_every_durable_failure_step`，
   要求六种内部 failure、所有 temp/descriptor close 路径与 directory sync
   存在；确认当前单一 `CKC_PROFILE_PLATFORM_ERROR` 无法通过。
3. 实现原子 header，机械替换 collector 的全部 32/64 位访问；用
   `cargo test --locked --test contracts profile_runtime_ -- --nocapture` 验证
   静态契约转绿。
4. 在 `tests/native/profile_generation.rs` 增加
   `profile_generation_should_publish_parseable_shard_after_durable_reopen`：构建
   instrumented dynamic library、执行 kernel、调用唯一 flush symbol、关闭
   library、通过 no-follow reopen 读取唯一 `.ckprof-part` 并
   `parse_profile_shard`。先确认至少一个旧平台 CI 反例仍由现实现覆盖。
5. 为 Linux x86-64/AArch64 分别冻结 stat offset，修复 syscall 返回值、
   no-replace collision 与每个 close；为 Darwin 使用不会生成额外 ABI 符号的
   identity 路径并保留 `renameatx_np(..., RENAME_EXCL)` 与双 fsync。
6. 添加 C harness fault tests，逐一注入 open/identity/create/write/file-sync/
   rename/directory-sync 失败，断言无 completed partial shard、temp 被清理、
   retry 或公开状态符合规范。运行 Native profile tests 转绿。
7. 运行格式、Werror compile probes、profile runtime object dependency audit 和
   全部 profile tests；把命令与计数写入
   `target/acceptance/v0.14/stage-12/`。

## 阶段命令

```sh
cargo test --locked --test contracts profile_runtime_ -- --nocapture
cargo test --all-features --locked --test native profile_generation_ -- --nocapture
cargo test --all-features --locked --test native profile_runtime_ -- --nocapture
cargo test --all-features --locked --test native artifacts -- --nocapture
```

## 边界

- 不用 mutex、heap-backed fallback 或非 lock-free 64 位 CAS 代替计数器原子。
- 不删除 file/directory durability barrier，不把 collision 当成功。
- 平台内部诊断可细化，但公开 Runtime ABI 和状态码不变。
