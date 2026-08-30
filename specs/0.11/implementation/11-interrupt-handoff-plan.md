# I23：Unix run 中断交接 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans，完全行内执行；用户禁止子代理。每项先 red、再 green，不跳过验收。

**Goal:** 修复已安装 SIGINT handler 到登记 private child 之间的丢信号窗口，保留 `245/CKR0006` 与所有现有门槛。

**Architecture:** 把现有 Unix interrupt 模块原样分离为私有源文件，供 CLI 与隔离回归共同编译。使用一个 lock-free AtomicI32 表示 unarmed/pending/child PID；handler 与登记通过同一个原子状态交接，不用锁、分配、第二个 pending 原子或生产测试开关。

**Tech Stack:** Rust 1.90 std、Unix signal/kill、现有 Native integration driver；Windows 实现不变。

## 已复诊证据与边界

- `33302635528/99233477608` 在 11:55:43Z 开始 Native suite；live UI 显示唯一未结束的
  `run::public_run_should_forward_interrupt_and_map_abnormal_child_to_ckr0006` 已运行超过 60 秒。
  旧同平台 `99232169032` 全 Native suite 仅 24 秒。当前未伪称远程已终态失败。
- 对 `5895242` 的原 Unix 模块逐字复制，仅添加隔离 harness；安装 handler、spawn
  自有 sleep、真实 SIGINT、set_child 的 before-arm 顺序在 2 秒期限后失败；after-arm 对照
  以 signal=2 通过。harness SHA-256 为
  `babc0c4eac2a1cef85c6ebd7b121ba8231c1e34149ace421404b6bc1c6f21a01`。
  red / control 日志为
  `179ada2373e1b547ca2f284247e4f047e7caa97e4a3f38bff0442d27a3808e41` /
  `a4a8803c6522adde5a61b3f5d278fff74b9db800cbb73173eda15619e0aab6bb`。
- 外部观察到 OS child 不等于父进程已经从 spawn 返回并登记 PID；原 handler 读取 0 时
  丢弃 SIGINT，原测试随后的 wait_with_output 无期限。复现证实产品漏洞，远端因果推断
  与现象相符，但没有声称抓到了远端进程栈。
- 保留现有语言/ABI/退出码、Windows 控制处理、性能协议、全部 10 required jobs；
  不通过 sleep 后再发信号、重发直到成功、ignore test 或放宽期望来掩盖故障。

## Task 0：先提交复诊、计划、验收补充

**Files:** 本文、`00-master-control.md`、`11-release-candidate-task.md`、
`11-release-candidate-acceptance.md`、`../review/implementation-blockers-01.md`。

- [x] 自审交接状态图、测试隔离与超时清理，确认不改变产品契约。
- [x] 运行 `git diff --check` 与 `cargo +1.90.0 test --locked --test contracts docs::`；
  文档契约 16 passed / 0 failed / 0 ignored。
- [ ] 仅提交上述文档，之后才改源码。

## Task 1：真实生产模块的确定性 red

**Files:**
- Create `src/cli/run/interrupt_unix.rs`：原 Unix 模块内部代码原样移动，不修复。
- Modify `src/cli/run.rs`：Unix 模块换成 `#[path = "run/interrupt_unix.rs"] mod interrupt;`。
- Modify `tests/native/run.rs`：用 path 引入同一私有源文件，不复制 handler。

- [ ] 添加隔离进程回归：当前 Native test executable 以 `--exact` 运行自身的目标测试；
  仅在该子测试进程安装 handler，使用真实 SIGINT 和真实自有 sleep child。标准 raise
  定向当前线程，避免测试 harness 的其他线程先后调度影响 before-arm 判定。
- [ ] 每个隔离 worker 放入自有 process group；RAII 在异常/超时时仅终止该 group 并回收
  自有进程。sleep handle 也以 RAII kill/wait，正常完成的子进程不能泄漏。
- [ ] 覆盖登记前、登记后、登记前两次 SIGINT、pending guard 丢弃后重新安装无信号四种。
  核心 before-arm 期望如下（不是 mock）：

```rust
let mut guard = interrupt::ForwardGuard::install().expect("install");
let mut child = owned_sleep();
assert_eq!(unsafe { raise(2) }, 0);
guard.set_child(child.id()).expect("arm");
assert_eq!(bounded_child_status(&mut child).signal(), Some(2));
```

  after-arm 将 raise 放在 set_child 后；重复 case 调两次 raise，仍要求 SIGINT 终止；
  drop case 在 spawn 前 raise/drop，再安装新 guard、登记 child，确认未被旧 pending
  终止后才发新的 SIGINT，并要求 signal=2。
- [ ] 执行 `cargo +1.90.0 test --all-features --locked --test native interrupt_handoff -- --nocapture`；
  观察 before-arm/重复 case 因 SIGINT 丢失失败，after-arm/guard-reset 对照通过。保留 red
  原始日志与摘要。不得让回归无限等待或影响 test runner 的 signal handler。

## Task 2：单原子交接最小修复

**Files:** `src/cli/run/interrupt_unix.rs`。

- [ ] 定义 `PENDING: i32 = -1`。install/drop 保持清空状态和恢复原 handler。
- [ ] 登记采用一个 swap，只有看到 pending 才转发缓存的 SIGINT：

```rust
if CHILD.swap(child, Ordering::AcqRel) == PENDING {
    forward_interrupt(SIGINT);
}
```

- [ ] handler 原子交接逻辑如下；只对正 PID 调用 async-signal-safe kill：

```rust
let child = match CHILD.compare_exchange(0, PENDING, Ordering::AcqRel, Ordering::Acquire) {
    Ok(_) => return,
    Err(child) => child,
};
if child > 0 {
    unsafe { kill(child, signal_number); }
}
```

  CAS 成功表示登记线程将负责待处理信号；若登记先赢则 CAS 返回正 PID，由 handler 转发。
  已 pending 时保持 pending；不自旋、不等另一个线程，不创建“检查后覆盖”的新丢失窗口。
- [ ] 同一 targeted regression 必须全绿；原完整 public run 中断测试仍要求 code=245、
  无 parent signal、stdout 空、stderr 精确 CKR0006。
- [ ] 行内检查 signal handler 只使用 lock-free 原子及 kill，不增加分配/锁/日志；
  pending 不跨 guard 泄漏，Windows 模块字节不变。

## Task 3：给现有 public integration 增加失败上界

**Files:** `tests/native/run.rs`。

- [ ] 将 public parent 放入独立 process group，使用同一 RAII owner；保留现有“child 已存在”
  条件和只发送一次 SIGINT，不增加固定等待、重新发送或 mock。
- [ ] spawn 等待和中断后退出等待都有独立 10 秒期限；超时明确 panic，owner kill/reap
  测试私有 group。读取 stdout/stderr 前回收遗留 descendants，避免继承 pipe 再次卡住。
- [ ] 使用仍存活的隔离 worker 验证 timeout cleanup 的反例：到期必须失败且不泄漏 child；
  此项只验证测试设施，不能替代产品行为回归。
- [ ] 再跑整个 `cargo +1.90.0 test --all-features --locked --test native run:: -- --nocapture`。

## Task 4：完整验收与提交

**Files:** review、本计划、阶段 11 acceptance；正式双语文档只在产品契约文字确需澄清时同步改。

- [ ] 执行阶段 11 原 default/all-feature/Clippy/fmt/release、generated/mutation/fact-audit、
  artifact/JIT 检查；保留 0 failed/ignored 和确切工具链身份。
- [ ] 新实现 SHA 做第一次完整 schema-6 性能门及 checker，保留首次原件、全部门槛不变。
- [ ] 提交实现与本地证据。等待原两个 Windows cold oracle 构建保存合格缓存，不因这个
  Darwin 问题取消整个旧 workflow。推送 feature branch，显式触发新 SHA 完整 10 jobs。
- [ ] 所有 required jobs 在同一最终 SHA 通过才关闭 I23/I21/I22 和阶段 11；随后才做
  `99-final-acceptance.md`。旧 589 部分证据保留，不和新 SHA 拼成全绿。

## 自审结论

本项是已有 run 契约的实现缺陷，不是优化/ABI 规范反例。单原子有两种先后次序，均能
将一次 pending 请求交给 child；非实时 SIGINT 允许多次未处理请求合并。测试子进程
隔离真实 handler，完整 public CLI 另外覆盖自拉起、状态映射和空 PATH。有限期限只把
原来的无限卡住转为明确失败，不降低验收。无需扩展语言设计或更改性能协议。
