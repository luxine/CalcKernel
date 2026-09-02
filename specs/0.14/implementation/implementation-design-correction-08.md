# 实施期设计复诊 08：最终 O3、收益隔离与 Schema 9 工具链闭包

## 阻断复诊

最终审计以真实候选、真实 oracle 命令和历史 replay 证据运行，确认了六个会阻断或错误放行发布的
问题：

1. 选中 Loop SIMD/unroll/SLP 后重放路径提前返回，跳过 v0.13 固定 DCE/cleanup 后缀；仅选中
   specialization/inlining 时又重新进入整个 ordinary O3，可能擅自应用计划外的收益优化；
2. tuning space 仍调用普通向量、展开和 SLP 静态收益门槛，导致本应交给测量决定的合法备选项不可见；
3. transaction 的 growth reject、普通 illegal 与 checker/compiler invariant failure 没有保持三种语义；
4. Schema 9 Oracle 虽声明空环境，却让 Clang/rustc 隐式从 PATH 找链接器；真实 Rust Oracle 在空环境
   必然以 `linker cc not found` 失败；
5. x86-64-v4 特性闭包漏掉 AVX-512CD，历史 v0.13 replay 也只检查返回码而未把报告、编译器与归档成员
   的精确身份互相绑定；
6. late alternative 物化所需的临时桥接状态摘要被错误写入 `Site`/`SiteAlternative`，破坏了 Schema 1
   冻结的 pre-tune 摘要等式，并使规范 `siteId` 自校验失败。

这些都是实现与既有冻结意图之间的可验证性缺口，不是性能门槛失败。修订不得改变语言/ABI、安全
语义、测量次数、性能阈值、搜索预算或 required CI 数量。

## 修订决议

- 重放先推进到固定 late-O3 边界，再按规范阶段物化 late alternative；任意非布局调优选择都不会
  重新进入 ordinary tunable phases，并始终执行固定 DCE/cleanup。布局仅在后缀完成后投影存活/
  新建基本块。
- tuning 专用 discovery/materializer/checker 只关闭 ordinary static-profitability 判定；legality、proof、
  target feature、transaction 与 structural growth 仍由独立 checker 完整重算。普通 O3 API 与阈值不变。
- growth reject 记录为 `growth-rejected`；合法性 reject 记录为 `illegal`；checker/compiler invariant failure
  立即作为 replay failure 失败关闭，绝不被搜索吞掉。
- Collector 独立解析 `/usr/bin/ld`，保留 `systemLinker` 字节身份。C Oracle 显式使用 Clang
  `--ld-path`；Rust Oracle 显式使用固定 Clang `-C linker` 与同一 `-C link-arg=--ld-path`。命令环境
  严格为空，辅助工具身份进入 `Command.inputs`，checker 复核现场路径、字节、完整 argv 与输入集合。
- x86-64-v4 必须精确具备 AVX-512 F/BW/CD/DQ/VL。v0.13 replay checker 的报告版本/SHA、candidate
  binary、顶层 replay compiler 和归档中的 `ckc-v0.13/ckc` 必须逐字节相同。
- Collector 在报告前删除 compile/profile/cache/lock scratch；checker 从全部 evidence `FileIdentity`
  反向计算唯一允许文件集。记录的 `Command.argv` 等于真实子进程 argv；需要安装布局的工具通过
  `executable` 选择等字节原映像，不伪造 argv 零元素。
- late alternative 可以在确定性桥接后的临时状态上完成候选发现与隔离物化，但该临时摘要不属于
  Frontier 线格式。每个 `Site.preStateDigest`、`SiteAlternative.preStateDigest` 与 `Unit.baselineStateDigest`
  均保持为同一个未改写的 pre-tune KIR 摘要；`siteId`、`unitId` 和 candidate-space digest 继续只按
  Schema 1 的规范材料导出。

## RED 与验收

- dead pure KIR 指令证明选中 Loop SIMD 后固定 DCE 仍执行；双调用 RED 证明只选一个 inline site
  不会自动应用另一个 ordinary inline；layout-only 去除元数据后等于普通 O3。
- 人为提高向量成本使 ordinary proposer 拒绝而 tuning space 仍物化并通过所有非收益检查。
- 单元回归分别锁定 growth/illegal/compiler 三种分类；跨类别冲突保留为 illegal expansion 并继续。
- Python 契约回归锁定空环境、显式 linker chain、toolchain 身份与 exact input set；真实 Clang/rustc
  空环境 probe 均成功生成动态库。
- Schema 9 mutation/static checks 锁定 AVX-512CD、历史 replay identity 和 evidence-root exact closure。
- Frontier 回归同时断言 site/alternative/unit 的 pre-tune 等式与 alternative/variant post-state 等式；
  真实 `tune build` 必须通过 decision decoder 的全部派生 ID 自校验。

所有既有发布阈值和 required gate 保持不变。
