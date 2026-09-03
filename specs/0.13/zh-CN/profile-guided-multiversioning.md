# CK 0.13 Profile-Guided Multiversioning 规范

[English](../profile-guided-multiversioning.md)

## 状态与权威性

本文是 CK 0.13.0 的预发布设计契约，基于 CK 0.12 final-candidate 源码树，
不声称当前编译器已经实现 PGO 或 runtime CPU dispatch。在本文实现、验收并发布
之前，CK 0.12 的语言、可观察语义、CLI、公开 Native C ABI、安全规则与优化器
契约仍然权威。

如果 0.12 candidate 在落地前发生变化，本设计必须先 rebase 到变更后的契约并重新
审查，之后才能继续实现。审查、实现阶段和验收证据不属于本文。

## 目标

CK 0.13 在不要求日常开发执行训练的前提下，引入真实 workload 信息和可移植 CPU
特化。编译器把 CK 自有静态事实与有界执行频率证据组合起来，为合格 Native Kernel
保留一个可移植 baseline，并只生成有收益的 feature variant。产物在运行时只解析
一次，选择编译器排序中当前 CPU 可兼容的最佳 variant，同时完整保留 CK 安全、严格
浮点、effect 与 ABI 语义。

本版本有五个互相关联的交付项：

1. 稳定、有界且由 CK 自有的 profile schema 与确定性身份；
2. profile 插桩、原子 shard、merge、inspect 与 use；
3. profile 引导的布局、inline、specialization、loop 与 SIMD 决策；
4. 有界 baseline+feature Native multiversion 与 runtime dispatch；
5. 语义、对抗、六 host、性能、体积与编译耗时门禁。

## 固定决策

- PGO 可选且默认关闭。`check`、`run`、普通 `build`、现有 test harness 与普通 release
  build 不需要训练，也不采集 profile。
- 0.13 不改变 CK 源码语法、无参数 `main()`、类型系统、严格 `f64`、checked
  first-error、effect、slice 语义或公开 ABI。
- CK 管理公开 `.ckprof` 格式、profile 身份、可信度规则和 KIR 优化决策。LLVM
  profile 格式只能是私有实现细节，不能作为 CK profile 输入。
- profile 观察结果只能判断收益，不能证明安全。训练中观察到的情况不能消除检查，
  也不能证明 alias、range、alignment、effect 或 bounds；推测快路径必须保留已验证
  guard 与通用 fallback。
- 插桩只存在于 profile-generation 临时产物。最终 profile-use 产物没有 counter、
  profile writer、输出路径或 profile 采集依赖。
- final profile 是终态聚合物。schema 1 merge 只接受完整 raw shard，因此同一次 recorded run
  不能藏在重叠嵌套聚合中被重复计数。
- `--cpu multiversion` 必须显式启用。每个合格 root 保留一个 ABI-compatible
  baseline，最多生成两个有收益的增强 variant；普通 `baseline` 与 `native` 继续保持
  0.12 含义。
- runtime CPU detection fail-closed。未知、矛盾、不可用或不支持的 feature 信息一律
  选择 baseline，不能乐观选择增强 variant。
- exported function 的公开地址始终是稳定 dispatcher thunk。variant 与支持符号隐藏，
  不属于公开 ABI。
- PGO use 支持 O2/O3；O2 的频率只通过 CK-owned late machine-layout plan 使用，不发出
  profile-derived LLVM metadata。PGO-influenced inline、specialization、loop cloning 与
  multiversion 只属于 O3。
  profile generation 使用一个固定且版本化的 instrumentation pipeline。
- 0.13 中 contract sanitizer 与 profile generate、profile use、multiversion
  不兼容；非法组合明确失败，不能静默改变策略。
- 0.13 PGO/multiversion 只支持 Native consumer。C、WebAssembly、默认 KIR
  inspection、公开 JIT API 和 cross-compilation 不获得隐藏 profile/dispatch 行为。
- 本版本没有 runtime adaptive recompilation、source SIMD、fast math、浮点
  reassociation、workload 猜测或搜索式 Auto-Tuning；offline Auto-Tuning 留到 0.14。

## 现有基础与版本边界

CK 0.13 复用而不替换 0.11/0.12 的以下基础：

- canonical SSA KIR、Memory SSA、region identity、effect summary 和独立
  fact/proof checker；
- scalar range、congruence、known-bit、alias、alignment、slice 与 contract fact；
- 确定性 transaction、audit ledger、cost unit 与有界 rewrite budget；
- canonical loop、dependence analysis、specialization、unroll、SLP、Loop SIMD、
  scalar fallback 与 target optimization profile；
- 固定 LLVM 22.1.8 Native bridge、ORC/JITLink runtime、lld artifact 路径、cache
  identity 与六个 release host。

不得为使 0.13 candidate 通过而降低任何 0.12 门槛。PGO 或 target variant 必须从
相同 verified pre-transform KIR 出发，并通过普通 O3 产物相同的 structural、proof、
effect、failure-order 与 backend audit。

CK 当前没有 function-pointer call 语言表面，因此 0.13 只支持 direct call。不得在本
版本虚构 indirect-call target profiling/promotion；function pointer 需要单独语言与 ABI
设计。

## 备选架构

### 选择：CK 自有 profile 并在 KIR 应用

CK 定义 instrumentation site 与 `.ckprof`，对 canonical pre-profile KIR 验证，执行
有界 KIR 决策，并在 lowering 后输出已验证 LLVM attribute/metadata。公开契约不依赖
LLVM，运行频率也能与 CK 独有的 range、alias、alignment、slice、effect 信息组合。

### 拒绝：公开 LLVM raw profile

直接透传 LLVM instrumentation 实现较快，但会把 CK 用户、cache、诊断与兼容性绑定
到固定 LLVM 格式，并把关键 site mapping 与策略放到 CK 独立 verifier 之外。

### 拒绝：adaptive JIT profiling

持续观察与重编译适应性更强，但增加 warm-up latency、内存、runtime 机制、非确定性，
并造成 executable/library 部署差异，不符合零额外依赖静态产物路线。

## 编译架构

Native O2/O3 流程变为：

```text
source -> checked program -> semantic MIR
       -> consumer/mode-specific scalar KIR
       -> canonical pre-profile KIR 与 site table
       -> 已验证静态分析与 0.12 scalar/vector 基础
       -> 可选的已验证 CK profile annotation
       -> 有界 PGO 决策
       -> 从同一 immutable pre-state 产生可选 multiversion plan
       -> 独立验证的 KIR variant
       -> per-variant LLVM module 与 PassBuilder pipeline
       -> dispatcher + artifact assembly
```

profile mode 只有三种：

- `off`：除可选 inspection 外不 materialize site table，codegen 不受插桩或 profile
  identity 影响；
- `generate`：固定 canonical pipeline 冻结 site identity 后插入 profile operation；
- `use`：重建同一 table，验证完整 profile，把 count 作为非 proof 分析 fact 导入，
  再运行 O2/O3。

profile counter operation 使用独立 KIR effect domain。它不能被删除、复制或移动越过
被计数事件，但不 alias CK program memory，也不制造虚假 Memory SSA barrier。
generation pipeline 只执行已证明保持 counted event 与 canonical site 一一对应的变换。

## CLI 与工作流

普通命令不变：

```text
ckc run app.ck
ckc build app.ck --out app
```

executable 便捷路径为：

```text
ckc pgo build app.ck --out app [--profile-out app.ckprof]
```

它事务性构建临时 instrumented executable，运行一次无参数 `main()`，验证并 merge
完整 shard，写最终 `.ckprof`，再构建 O3 正式产物。子进程继承当前工作目录、标准流与
环境。训练是真实执行而非 sandbox，其副作用就是用户明确要求运行的 workload 副作用。
非零退出、signal、缺失/空 shard、写失败、profile 错误或正式构建错误都不能留下最终
artifact。

CK 0.13 不向源码 `main()` 暴露 `argv`，所以便捷命令不能假装命令行输入会进入 CK。
executable 必须由自己的 `main()` 执行代表性 workload。library 与多 workload 使用
显式流程和用户自有 host/test harness：

```text
mkdir profiles

ckc build kernels.ck --kind dynamic --out kernels-profiled \
  --pgo-generate profiles/ --cpu multiversion

# 一个或多个 host/test run 加载 kernels-profiled、执行真实 workload、
# 等待 CK 调用静止，然后调用生成的 ck_profile_flush_* control entry。

ckc pgo merge profiles/ --out kernels.ckprof

ckc build kernels.ck --kind dynamic --out kernels \
  --pgo-use kernels.ckprof --cpu multiversion -O3
```

只读 inspection：

```text
ckc pgo inspect kernels.ckprof
ckc pgo inspect kernels.ckprof --json
```

CLI 契约：

- `--pgo-generate <directory>` 与 `--pgo-use <file>` 互斥；
- profile generation 拒绝 O0/O1，使用固定 generation pipeline；
- profile use 接受 O2/O3，`--cpu multiversion` 要求 O3；
- `--pgo-generate` 只由 Native `build` 的 executable、dynamic 与 static artifact 接受。
  object generation 被拒绝，因为未链接 object 没有确定的 process/library lifetime 或
  flush owner；
- `--pgo-use` 由 Native `build` 和 Native `emit-kir` 接受。0.13 拒绝
  `--cpu multiversion --kind object`，因为 dispatcher 与 variant 是独立审计
  module，而 schema 1 未定义 multi-object bundle 或跨平台 partial-link product。
  single-version baseline/native profile-use object 仍受支持；
- Native executable 与 Native library 是两个 profile-topology class。dynamic、static 与
  object artifact 共享 Native-library topology，因此通过临时 dynamic/static library 生成的
  compatible profile 可用于 baseline/native object。Native `emit-kir` 按 `--consumer` 选择的
  topology 校验，而不是按 physical artifact kind 校验；
- Native `build` 与 Native `emit-kir` 的 `--cpu` 变为
  `baseline|native|multiversion`；portable consumer 与当前一样拒绝后两者；
- `ckc pgo build` 只接受有合法 `main()` 的 executable，默认 O3；library 使用显式流程；
- `--profile-out` 默认 `<out>.ckprof`，不能与最终 artifact 路径相同；
- `--pgo-generate` 的 `<directory>` 必须已存在、必须是真实 directory，resolved path 中
  不能有 symlink/reparse-point component。compiler 按 build-time current directory 解析并
  规范化 absolute path、捕获 platform file identity，并只把该 path/identity 嵌入临时
  generation artifact。operational path/directory identity 不进入 profile、final-artifact 或
  cache identity。runtime collector 使用 component-wise
  no-follow/reparse-point check 重新打开 directory，并把所有 temporary/completed file operation
  anchor 到该 verified directory handle；build 与 execution 之间发生替换时拒绝运行。
  platform/filesystem 若没有 stable directory identity，则拒绝 generation；
- generation 配合 `--cpu multiversion` 时绑定预期 target-set identity，但只执行一个
  instrumented baseline implementation；训练阶段不在已经优化好的 variant 间 dispatch；
- generate artifact 不进入 Native object cache，因为输出目录属于 operational state，
  不是可复现正式代码身份；
- deprecated `build-llvm` 不增加 PGO/multiversion。

现有 transactional output 规则把 final artifact、profile、header 与 sidecar 作为一个
output set；pre-commit 失败必须保留全部旧 destination。

## Profile 身份

`CkProfileIdentity` schema 1 精确包含：

- profile format 与 profile-contract schema identity；
- compiler package version、compiler source identity、profile runtime identity；
- canonical semantic module graph 与 canonical pre-profile KIR digest；
- 完整确定性 profile site-table digest；
- language、public Native ABI、Runtime ABI、KIR、proof、cost-model、target profile、
  LLVM bridge 与 cache schema identity；
- target triple、pointer width、endianness、object format 与 OS ABI；
- overflow、bounds、strict-float、sanitizer、consumer 与 profile-topology class
  （`native-executable` 或 `native-library`）；
- 会改变合法 consumer 的 optimization family（`o2`/`o3`），固定 generation topology
  单独表示；
- CPU policy 与完整有序 multiversion target-set digest；
- 全部固定 PGO confidence、profitability、code-growth 与 resource limit。

身份排除绝对路径、源码格式、注释、timestamp、PID、host name、shard name、环境变量、
机器负载与 physical dynamic/static/object artifact kind。physical kind 继续进入 final
artifact transaction 与 Native object/cache identity，但不能拆分 canonical Native-library
KIR/site topology 相同的 profile。格式/注释变化只有在 canonical semantic/KIR/site identity 字节完全一致时
才能复用。任何语义、module graph、site、compiler contract、ABI、target set、safety 或
schema 变化都会失效。

merge/use 逐字段比较，报告第一个稳定 field path 及 expected/observed digest。没有
`ignore mismatch`。baseline 与 multiversion target set、不同 target、安全 mode 或
compiler contract 的 profile 即使 counter 类似也不能合并。

## `.ckprof` 与 shard 格式

最终文件 magic 为 `CKPROF01`，完整 shard 为 `CKPART01`。二者均使用 canonical
big-endian integer、固定 numeric tag、length-prefixed UTF-8、lexicographic record 与
覆盖此前全部 canonical bytes 的 domain-separated SHA-256。未知 tag、重复 field、尾随
bytes、错误 digest 或非 canonical 顺序均为错误。

最终 profile 包含：

1. 完整 `CkProfileIdentity`；
2. canonical site descriptor table；
3. saturated aggregate counter record；
4. completed-run 与 merged-shard 数；
5. overflow 与 incomplete-observation flag；
6. final content digest。

site ID 为 canonical function identity、KIR location、site kind 与 kind descriptor 的
SHA-256 前 128 bit；完整 descriptor 与 site-table digest 才是权威。不同 descriptor
出现同一 128-bit ID 是 hard collision error，不能合并。

0.13 site kind 闭集为：

- function entry；
- 选中的 CFG edge；未插桩 edge 只能由已验证 spanning-tree equation 重建；
- loop trip-count histogram；
- 选定 call/loop/versioning decision 处的 slice-length histogram；
- 现有 comparison 或 direct call 的 candidate-constant hit/miss。

没有任意 value、memory address、string、byte array、file content 或 indirect-call-target
record。candidate constant 只记录 canonical site table 中的有界 ordinal，不把任意
runtime integer 复制进 profile。

counter 是 saturated `u64`。length/trip 是 `u32`，严格使用 16 个 bucket：`0`、`1`、
`2`、`3..4`、`5..8`、`9..16`、`17..32`、`33..64`、`65..128`、`129..256`、
`257..512`、`513..1024`、`1025..2048`、`2049..4096`、`4097..65536`、
`65537..u32::MAX`。一个 constant site 最多八个 candidate 加一个 `other`。

schema 1 resource limit 为：每 module 最多 1,048,576 sites、每次 merge 最多 4,096
input shards、每 histogram 16 buckets、每 site 8 candidate constants、每个输入或最终
profile 最大 512 MiB。allocation 前必须检查全部 size arithmetic。

loop observation 超过 `u32::MAX` 时进入最后一个 bucket，并设置 trip-saturation flag；
不能截断到较小 bucket。

saturated counter 或 histogram 不能用于建立 confidence/profitability；受影响 site 为
`unknown`。若 saturated value 参与 spanning-tree equation，则所有依赖它重建的 edge 也为
`unknown`。unsaturated equation 若不能重建出唯一、非负且内部一致的 count，则 shard/profile
malformed。merge 后以及 profile application 前都要再次检查这些规则。

## 插桩与 collection runtime

canonical profile topology 在插桩前冻结。function entry、确定性最小 CFG edge 集、loop
exit、选定 slice length 与 pre-existing constant candidate 获得显式 profile operation。
critical edge 在分配 ID 前确定性 split。topology 不依赖 hash-map order、wall time、
target load 或 LLVM 优化决策。

embedded generation runtime 在 target 保证时使用 process-local lock-free 64-bit counter。
update 为 relaxed 并检测 wrap；wrap 设置 overflow bit，序列化值为 saturated。没有所需
atomic primitive 的 target 拒绝 generation，不能生成有 data race 的插桩。counter storage
私有、不可导出，并与 CK-visible memory 分离。

instrumented executable 的 compiler-owned entry wrapper 在 `main()` 正常返回后、process
返回 OS 前写一个 shard。automatic workflow 只有在 child 自身 exit zero 时才接受该 shard。
异常终止可以留下 temp，但不能破坏完整 shard；automatic mode 对异常 child 失败。

instrumented static/dynamic library 则在生成的 header 以及适用的临时 export/import table
增加一个 instrumentation-only C control entry：
`ck_profile_flush_<full-profile-identity-hex>() -> i32`。
`full-profile-identity-hex` 是 canonical serialized `CkProfileIdentity` 的 SHA-256 所得 64 个
lowercase hexadecimal character，不是 serialized identity text 本身。该 entry 同时加入临时
export/import table。host 必须等待进入该 library 的调用静止，并在 host-defined shutdown
boundary 调用它：dynamic library unload 前，
或 final linked static-library state 被丢弃前。第一次调用 snapshot counter 并写恰好一个
shard；后续
调用 idempotent，返回同一 success/failure status。library unload hook 与 `DllMain` 不做
profile I/O，因此 write failure 可由 host 同步观察。返回 0 表示 validated completed shard
已 publish；稳定非零 instrumentation status 表示该 library instance 没有 publish completed
shard。普通 artifact 与 profile-use artifact
完全不含该 control entry、symbol 与 runtime，它也不进入 public Native ABI versioning。

host quiescence 后的 concurrent flush call 通过 private atomic state machine 串行化；恰好一个
call 执行 publication，所有 caller 观察相同 terminal status。仍有 thread 可能进入或执行 CK
code 时调用 flush，违反临时 instrumentation API precondition；host test seam 可观察时必须
diagnose，不能通过并发复制 counter 假装 data-race safe。

shard publication 在所选目录创建 unique temporary file，flush 并验证 bytes，再 atomic
rename 为完整 `.ckprof-part`，不能覆盖已有 entry。并发 process 不更新同一 file。
directory merge 忽略可识别 temp 并报告数量。

runtime 不发送 telemetry，也不访问网络。profile 仍可能暴露 aggregate control-flow，
必须按照 workload build artifact 保护；格式不主动记录 raw workload data。

## Merge、inspection 与权重语义

`ckc pgo merge` 只接受完整 `.ckprof-part` shard；最终 `.ckprof` 是 terminal aggregate，
schema 1 拒绝将它作为 merge input。命令扫描显式 shard file 与显式 directory 一层内容，
按 canonical content identity 排序，验证全部 bytes/identity，拒绝 symlink 与重复 run ID，
然后 saturated add；directory 不递归。重复 input 报错，不能意外双倍加权。重新加权必须
保留并重新选择 raw shard；嵌套或重叠 aggregate profile 不能静默重复计算某次 run。

count 精确求和，不做隐藏 per-run normalization 或推断重要性。workload 运行十次就贡献
十次执行；用户用自己选择的输入与重复次数表示 workload mixture。merge 输出排除 shard
UUID、文件名、时间与输入顺序；同一 validated shard set 产生 byte-identical `.ckprof`。

`inspect` 使用相同 untrusted-input parser，报告 identity、site coverage、run/shard count、
saturated site、histogram、hotness summary 与当前编译器 compatibility。JSON schema
版本化且确定性；inspection 不能使不兼容 profile 变为可用。

## Confidence 与 hotness model

profile count 是被记录 run 的精确观察，不是普遍事实。schema 1 固定 integer 规则：

- decision site 至少 128 observations 才能引导代码复制、cold marking 或强 LLVM
  likelihood；
- branch/constant 达到 90% 才是 dominant；
- trip/length bucket 达到 85% 才是 dominant；
- function 至少 128 entries 且 block 不超过其 1% 才能标记 cold；
- zero observation 不能证明 unreachable，也不能授权删除。

estimated dynamic work 使用 saturated `u128`，结合 entry/edge/loop count 与 immutable
target-profile static cost unit。按 work 降序并以稳定 function identity 打破平局，选择
覆盖 module 90% estimated work 的 PGO-hot root；除非是唯一 eligible root，每个选中 root
至少占 module work 1%。

低于 confidence 的 site 保持普通静态优化器决策。任何阈值变化都推进 profile-contract 与
cache identity。

每个 profile-weighted proposal 暴露一个闭合 observed outcome class 集与 immutable integer
target-cost formula。`N` 是全部 class count 的 checked sum。对 exact branch/value class，
checker 计算 unchanged baseline 与 guarded selected path 的 integer cost difference；miss 包含
完整 generic fallback。对 histogram bucket，checker 必须使用 closed target formula 证明
bucket 内每个 `v` 的 `baseline_cost(v) - selected_cost(v)` signed lower bound；无法证明时，
该 bucket 不提供 PGO authority，proposal 回退到 static decision。不能发明 sampled 或
representative value。

conservative net benefit 是 `class_count*lower_bound_difference` 的 checked signed-magnitude
sum，再减 `N*guard_cost`。proposal 必须在该 lower bound 以及全部现有 static/growth gate 下
仍有收益。全部 magnitude 使用 checked `u128`；overflow、indeterminate sign、tie 或 fractional
ambiguity 一律选择 baseline。ratio 使用 checked cross multiplication。independent checker
从 profile record 与 target formula 重算每个 class bound 与 total，不能信任 proposal total。
若 saturated site 参与 function dynamic-work estimate，该 function 不可成为 PGO-hot root。

## PGO 引导优化

O2 中 validated count 只能影响非复制决策：

- late machine-block order，不能 duplicate body 或改变 semantic machine CFG；
- function 与 hot/cold section order；
- accepted order 所必需的 terminator inversion/fallthrough repair、target branch relaxation
  与 alignment padding。

O2 phase boundary 为闭集。profile-on/off build lower 相同 semantic/structural KIR；O2
profile analysis 作为 unlowered sidecar 保留到 late boundary。两种 mode 以 profile-blind
方式运行完整 default O2 LLVM IR pipeline，以及全部 ordinary IR preparation、instruction selection、
scheduling、outlining、splitting、merging、tail duplication 与其他 machine-structure pass。
该 boundary 之前不存在 profile summary、entry count、branch weight、hot/cold attribute、
CFG successor order 或其他 profile-derived LLVM input。

bridge snapshot 并验证所得 machine CFG、block body 与 symbol map，再应用一个 CK-owned
`CkLateProfileLayout` plan。该 pass 只能 permute 现有 machine block/function/section，并修复
permutation 所需 terminator/fallthrough；不能 duplicate/delete body、改变 non-terminator
instruction、outline、split、merge、reschedule 或改变 call target。此后只运行 target-mandated
branch relaxation、offset/fixup assignment、alignment padding 与 object emission，且都不接收
profile data。unmapped block 保持 ordinary order。verifier 独立比较 pre/post snapshot，拒绝
闭合集以外 delta。定义 O2 权限的是该 structural boundary，而不是 LLVM pass 名称。

每个 target 拥有 closed post-layout repair allowlist。若 CFI、unwind、LOH、security、bundle
或其他 target state 需要 allowlist 以外 repair，则拒绝 layout proposal 并保留 ordinary order。
AArch64 在接受 reorder 后重新运行所需 branch relaxation。该 conservative target fallback
属于正常 explanation，不能作为隐式扩张 allowlist 的许可。

O3 还可以影响已有 verified transformation：

- 从 trip histogram 选择 unroll 与 vector/interleave factor；
- 调整现有 bounded direct-call inliner cost，偏向 hot callee 并拒绝 cold size growth；
- 调整 runtime vector break-even threshold，但不能删除 scalar fallback；
- length bucket dominant 时选择 guarded short-slice path；
- 现有 source/KIR constant dominant 时，每 value site 最多一个 guarded PGO candidate；
- 以 estimated dynamic benefit 排序现有 specialization、Loop SIMD、SLP、versioning；
- 决定哪些 eligible root 值得生成 CPU variant。

PGO specialization 复用 0.12 specialization transaction、proof checker、每 original 最多
三个不同 clone 与 aggregate code-growth budget。它增加 runtime equality/range/length guard，
不匹配时调用原通用 body。recursive SCC、dispatcher 之外 exported-body cloning、ordered
effect、可能的 checked first failure、sanitizer mode 与 unsupported consumer 继续保守拒绝。

profile weight 不能开启 fast math、contraction、reassociation、speculative memory access、
widened footprint、unchecked pointer arithmetic、effect motion 或 guard elimination；只有
静态 CK proof 能授权这些行为。

## PGO pipeline 顺序

O3 profile-use pipeline：

1. 构造并验证 target set 与 canonical pre-profile KIR；
2. 重建完整 site table 并验证 profile identity；
3. 附加 immutable non-proof profile analysis，计算 confidence/work；
4. 运行现有 O1 prefix 与 profile-weighted direct-call specialization；
5. 运行现有 O2 inline、Memory SSA、GVN、forwarding、DSE、cleanup，使用 profile-aware
   bounded inline cost；
6. canonicalize loop 并重建 dominance、effect、range、Memory SSA、dependence；
7. 从一个 immutable scalar pre-state 提出 PGO length/value fast path 和现有
   unroll/SLP/Loop SIMD alternative；
8. 独立验证每个 proposal，在现有安全和新 growth budget 内事务性选择 estimated dynamic
   cost 最低者；
9. 冻结 verified baseline module，从同一 logical pre-state 提出 target variant，不能从
   另一 variant 派生；
10. 每个 accepted module 单独 lowering，附加 checked frequency metadata，运行对应 LLVM
    pipeline，审计 feature 并组装 dispatch；
11. commit 前执行最终 structural、proof、profile-mapping、symbol、artifact、cache 与
    determinism validation。

任何 CFG-changing step 都使旧 graph profile mapping 失效。只有 closed、独立检查的
mapping record 才能 transfer count，否则受影响 site 变 unknown，不能猜测。

## Multiversion target set

`KirMultiversionTargetSet` schema 1 包含一个 baseline target profile 与有序闭集 enhanced
feature profile。每项记录 target triple、CPU/feature string、data layout、KIR operation/
cost profile、LLVM/bridge identity、runtime detection predicate 与 SHA-256 digest。所有
variant 使用相同 public ABI 与 source safety mode。

初始 target table 使用 feature level，不猜测 microarchitecture：

- x86-64 Linux/Darwin/Windows：ABI baseline、`x86-64-v3`、`x86-64-v4`；v3 要求
  完整 v3 feature 与 OS AVX/YMM state，v4 还要求完整 v4 AVX-512 与 OS opmask/ZMM state；
- AArch64 Linux：ABI Armv8-A Advanced-SIMD baseline、SVE、SVE2；SVE2 implies SVE，
  OS 必须声明可用 SVE state；
- AArch64 Darwin/Windows：schema 1 仅 baseline，因为 0.13 不拥有经审查的可移植 SVE
  feature/state query。

SVE/SVE2 profile 仍只暴露 0.12/0.13 定义的 fixed-width vector KIR operation。LLVM 可以
合法地把这些内部 fixed operation lower 为 SVE instruction，但 0.13 不增加 scalable KIR
value 或公开 ABI。

baseline-only target set 合法，并输出稳定 `no-compatible-enhanced-tier` explanation。
需要一个本地 Apple/其他 CPU exact model 时继续使用 `--cpu native`。x86 level feature list
与 AArch64 HWCAP mapping 是随 LLVM 22.1.8 固定的 compiler-owned canonical table；表变化
推进 target-set schema。

feature level 更高不代表一定更快。编译器建立该 level 的 target cost profile，只提出合法
变换并按 root 预测成本排序；root 可以优先 v3 而不是 v4，也可以优先 baseline。runtime
使用这个 per-root order，而不是数字最大的 feature level。

## Multiversion eligibility 与 budget

eligible root 是 exported CK function 或 executable entry，其 reachable optimized body
非 recursive、Native-supported，并至少存在一个 target-dependent plan，同时预测降低 10%
执行成本与两个 absolute target cost units。hidden direct helper 可在 root variant 内 clone
或 inline，但不独立 export。

有 valid profile 时只提出 PGO-hot eligible root；没有 PGO 时，`--cpu multiversion` 使用
普通静态 target cost 与稳定 root order。二者共同遵循：

- 一个 root 精确包含一个 baseline 与零至两个 accepted enhanced variant；
- 每个 enhanced variant 从同一 verified logical KIR pre-state 开始；
- 每个 variant 有独立 target-profile digest、proof root、cost、code size、feature audit；
- multiversion 额外 KIR units 不能超过完整 post-O3 baseline module units，因此最终 module
  KIR 最多为 baseline 两倍；
- PGO specialization 共享而不是重置全部 0.12 clone/transaction budget；
- budget exhaustion 或收益不足保留 baseline，并记录稳定保守原因。

candidate total order 为：estimated dynamic cost、更小 code size、更少 required feature、
target-tier identity、root/function identity。rejected trial 不返还 audit budget。

## Runtime dispatch 与公开 ABI

accepted exported root 的原 public symbol 指向 baseline-safe dispatcher thunk。thunk 保持
精确 platform C ABI、checked status/result-slot、slice flattening、alignment、unwind policy
与 symbol visibility。implementation symbol 含 content digest，并从 header、export table 与
普通 symbol lookup 隐藏。

first call 获取一次 process-local normalized capability bitset，按 root variant 排序选择，
再用 acquire/release atomic 发布 function pointer。并发 first call 可以计算同一答案，但只能
发布 compatible verified pointer。后续调用执行一次 atomic load 与 indirect tail call，不再
执行 CPUID/HWCAP query。public function address 始终是 thunk。

x86-64 使用 compiler-owned CPUID/XGETBV，同时要求 hardware bit 与 OS register-state。
AArch64 Linux 使用启动 auxiliary-vector HWCAP/HWCAP2，不解析可变文本。unsupported OS/
architecture 只提供 baseline。query failure、heterogeneous uncertainty、malformed state 或
未知未来 bit 一律 baseline。

production artifact 没有可以强制不支持 feature 的 environment variable/public API。测试
只能把 private detector seam 链进 test fixture。static archive 用 target-set digest namespace
私有符号；dynamic library/executable 隐藏它们。resolver 与 thunk 按 baseline 编译，并审计
不能含 optional instruction。

## Native LLVM 与 artifact 契约

baseline、每个 enhanced variant 和 dispatch support 分别 lowering 到带精确 target attribute
的 LLVM module。0.13 禁止 cross-variant LTO，防止 optional instruction 泄露进 baseline 或
dispatcher。artifact assembler 只在每个 module 通过 LLVM verify 与 feature containment
disassembly 后链接。

O2 中 CK 只把 validated count 转为上述 private late-layout plan；LLVM 不接收 profile-derived
metadata/attribute。O3 中，CK 验证 exact KIR-to-LLVM block/function map 后，可附加 LLVM
branch weight、entry count、hot/cold attribute 与 internal profile summary，供 inline、vector
cleanup、scheduling、instruction selection 与其他 O3 transform 使用。两种 mode 都不能让
LLVM 削弱 CK alias、bounds、failure、floating 语义。

Native bridge 增加 explicit feature-level target machine、normalized runtime predicate、
verified O2 late-layout boundary、O3 profile metadata attachment 与 per-module feature audit。
全部 query cost/operation 仍遵守 0.12 closed target-profile validation。

最终 executable、dynamic、static 与受支持的 single-version object 继续遵守现有
system-runtime policy 自包含。multiversion final artifact 只可为 executable、dynamic 或
static；被拒绝的 single-object 组合不能静默 repack。profile-use/ordinary artifact 不能
import CK profile writer、LLVM profile runtime、compiler library 或新 non-system shared
library。只有临时 instrumented artifact 含私有采集 runtime。

## 兼容性、schema 与 cache identity

public Native C ABI 保持 1，Runtime ABI 保持 2，因为 profile/dispatch helper 是 private
compiler support，不是 CK-callable runtime API。KIR 推进到 schema 3，容纳 profile operation、
immutable profile annotation、multiversion bundle、dispatch plan。target profile、proof、
optimization audit、private profile-runtime、private dispatch-runtime schema 在实现时明确
推进。private LLVM bridge 从 ABI 3 推进到 ABI 4。

Native cache 从 `CKCOBJ02`、key schema 3、manifest schema 3 推进到 `CKCOBJ03`、key
schema 4、manifest schema 4。除全部 0.12 字段外，还覆盖：

- profile mode、profile format/contract identity、exact `.ckprof` digest；
- physical output artifact kind 及其 validated profile-topology compatibility；
- 全部 confidence、hotness、weighting、site、PGO cost constant；
- target-set 与 per-variant profile/proof/codegen digest；
- dispatch table、detector、thunk、private runtime identity；
- 全部 multiversion profitability 与 code-growth budget。

generate-mode artifact 不缓存。profile-use variant 可以分别缓存，但 bundle hit 只有在
dispatcher manifest 与全部 referenced variant object 都通过验证时才接受；缺失、多余、
重排、redirect 或 mismatch variant 拒绝整个 hit。

相同 canonical source、compiler/toolchain、flag、target set、`.ckprof` bytes 必须生成可
复现 final KIR、explanation、variant order、object 与 artifact bytes（平台签名边界除外）。
profile timestamp、shard order、local path、build-host CPU、map iteration 不能影响
profile-use artifact。

## 错误、安全与隐私

profile file/directory 都是 untrusted input。parser 在暴露 record 给 optimizer 前验证 magic、
version、length、count、canonical order、duplicate identity、integer arithmetic、resource
limit 与 digest；merge/transactional output 不跟随 symlink，也不按未检查 length 分配。

稳定 diagnostic category 区分 invalid CLI combination、generation runtime failure、
malformed shard/profile、identity mismatch、unsupported target set、insufficient observation、
invalid profile-to-KIR mapping、detector construction failure、variant verification failure、
artifact feature leakage。malformed identity/mapping/proof/feature containment 是 compiler
error，不产生 output/cache。low confidence 与 insufficient profitability 是有 explanation 的
正常 baseline fallback。

没有命令上传 profile、source、counter 或 diagnostic。格式省略 raw workload value 与 local
path，但 aggregate control flow 本身也可能敏感；文档必须要求用户像 benchmark/build data
一样保护 `.ckprof`，不要意外公开。

## Inspection 与 explanation

`--explain-optimization` 的确定性报告增加：

- profile identity/digest、coverage、confidence、ignored-site reason；
- function dynamic-work rank 与 selected hot root；
- branch/layout/inline decision 与 exact supporting counter ID；
- PGO value/length candidate、guard、fallback、cost、proof、rejection；
- multiversion target set、considered tier、required feature、predicted cost、code growth、
  accepted order、dispatcher identity；
- cache use、profile mapping transfer、budget exhaustion、conservative fallback。

Native `emit-kir --cpu multiversion` 打印 target-set identity、verified baseline、accepted
variant KIR module、dispatch plan 与 hidden symbol map。不能因为 inspection machine 缺少某
feature 就 resolve host 或隐藏 variant。

## 验证策略

验收必须包含以下全部内容，不允许 ignored test 或降低门槛：

1. 全部 0.12 language、ABI、optimizer、runtime、artifact、sanitizer、differential、mutation、
   performance 与 six-host contract 继续全绿。
2. golden/mutation tests 覆盖 canonical profile bytes、site stability、comment/format reuse、
   每种 identity mismatch、hash collision、malformed length/tag/order/digest、duplicate shard、
   final-as-merge-input rejection、counter/equation saturation、resource limit、deterministic
   merge/JSON inspect。
3. instrumentation tests 对 normal executable exit、early return、break/continue、checked
   failure、recursion、multi-threaded host call、multiple process、host-quiesced library flush、
   concurrent/repeat-flush idempotence、unload-without-I/O、write failure propagation、abnormal termination
   验证 exact function/edge/loop/length/constant count。
4. differential tests 在 training、held-out、adversarial non-training input 上比较 ordinary O0、
   ordinary O3、generate execution、PGO O2/O3、baseline、每个 test-only forced compatible
   variant 与 production dispatch。
5. mutation tests 证明 profile data 不能单独删除检查、扩大 memory footprint、改变 first-error、
   reorder print/effect、开启 fast math、forge KIR mapping、超出 code budget 或选择不支持
   feature variant。
6. artifact-matrix、object/disassembly audit 证明被拒绝的 generation 与 multiversion-object
   组合在输出前失败；baseline/thunk 无 optional ISA，每个 variant 只含 declared feature，
   variant/runtime symbol 隐藏，final-use artifact 无 profile runtime，ordinary/profile-use
   public ABI/header bytes 稳定；generation-only flush declaration 只能存在于临时
   instrumentation header。通过 dynamic/static packaging 生成的 Native-library profile 必须
   可用于 baseline/native object；executable-topology profile 必须拒绝该 use。
7. runtime dispatch tests 覆盖 concurrent first call、stable public address、exactly-once
   capability caching、per-root order、query failure、baseline-only target 与可用 real hardware。
8. reproducibility tests 在不同 directory、shard、map、process order 构建/merge，要求
   byte-identical final profile 与 unsigned artifact。
9. O2 phase-boundary mutation test 注入 profile 偏好的 inline/vector/CFG/tail-duplication
   opportunity，证明 profile-on/off snapshot 在 `CkLateProfileLayout` 前完全一致；随后用
   MIR/object/disassembly audit 证明 accepted delta 仅含 ordering、必要 terminator/fallthrough
   repair、branch relaxation 与 alignment padding，并且不存在 profile-derived LLVM metadata。
10. exact final candidate SHA 通过 quality、Native integration、六个 Native host 与固定
   x86-64/AArch64 performance acceptance。

## 性能、体积与编译耗时契约

benchmark report 推进到新版本 schema，固定 exact 0.13 candidate、0.12 replay compiler、
LLVM/Clang 22.1.8、Rust 1.90.0、source、training shard、final profile、target set、variant
object、sampling order、hardware identity 与全部 digest。training input 与 held-out measurement
input 分开固定；correctness 覆盖二者及 adversarial input，PGO timed result 只使用 held-out。
改变 workload、exclusion、threshold、rerun policy 都是 reviewed contract change。

稳定 x86-64/AArch64 worker 必须满足：

- ordinary no-PGO baseline/native 相对 exact 0.12 replay 的 throughput geometric mean 退化
  不超过 2%，单项不超过 5%；
- declared PGO-sensitive suite 中，PGO-use 相对相同 0.13 no-PGO CPU policy 的 geometric
  mean throughput 至少提升 5%，held-out 单项不允许慢 3% 以上；
- feature-eligible multiversion suite 中，在具备 required enhanced tier 的 worker 上，
  dispatched artifact 相对 portable baseline 的 geometric mean 至少提升 8%，单项不允许
  慢 3% 以上；
- resolver 完成后，dispatch throughput 的 geometric mean 至少达到相同 selected tier direct
  artifact 的 98%，单项不允许慢 5% 以上；
- combined PGO+multiversion 相对适用的 PGO-only/multiversion-only 较快者，geometric mean
  不允许慢 2% 以上，单项不允许慢 5% 以上；
- 相对使用同一 training/evaluation split 与 safety/float precondition 的等价 pinned
  Clang-PGO/Rust-PGO oracle，CK geometric mean 至少达到 95%，每个 accepted kernel 至少
  90%，并继续满足累计 0.12 hand-SIMD/domain gate；
- 固定 instrumentation corpus 中 generation execution 不超过 ordinary artifact 的 5 倍；
  这是 tooling bound，不是 final artifact runtime allowance；
- PGO-use single-version source-to-object 编译 geometric mean 不超过对应 ordinary 0.13
  baseline 的 1.5 倍，multiversion 不超过 2.5 倍，combined 不超过 3.5 倍；单项上限分别为
  2、3、4 倍；样本使用已终止子进程的 user+system CPU time，排除托管 worker 被调度移出的
  时间，同时不移除任何编译器工作；
- PGO-only aggregate artifact size 不超过 ordinary 的 1.25 倍、单项不超过 1.5 倍；
  multiversion/combined aggregate 不超过 2 倍、单项不超过 2.5 倍；
- 每个 host 等价 stripping/signing 边界后的 distributed `ckc` archive 相对 exact 0.12
  不超过 15%。

全部 timed channel 保持固定 warm-up、rotating order、upper median、stability、equivalence、
fail-fast。CPU detection、dynamic loading、symbol resolution 在 steady-state timed call 前完成，
另有 untimed record 证明 resolver 只运行一次。stability 失败代表 evidence 无效，不能重跑到
得到有利样本。

## CI 与 release 验收

CK 0.13 不新增一套完整 pinned-LLVM bootstrap matrix，而扩展现有十 job 契约：

- quality 负责无需 Native toolchain 的 format/schema/unit/mutation/document/cache；
- Native integration 负责真实 instrumentation、merge/use、final artifact 与 profile-runtime
  audit；
- Darwin ARM64/x64、Linux ARM64/x64、Windows ARM64/x64 负责 host correctness、ABI、
  baseline fallback、object format 与所支持 real detector；
- 稳定 x86-64/AArch64 performance worker 负责固定 PGO/multiversion training、held-out
  performance、feature containment、size、compile-time evidence。

全部 job 复用现有 verified LLVM/Clang manifest/cache。required-feature worker 在 measurement
前发布 exact capability manifest；缺少 required tier 必须失败，不能静默 skip。最终验收绑定
一个 exact candidate SHA 和可下载、带 hash 的 evidence。

## 完成与未来边界

只有 exact candidate SHA 满足上述全部 semantic、profile、security、structural、dispatch、
ABI、artifact、cache、reproducibility、performance、size、compile-time 与 six-host gate，且
当前英文/中文规范与实现一致，CK 0.13 才算完成。

以下留在 0.13 之外：source function pointer 与 indirect-call PGO、source SIMD/intrinsic、
fast math、浮点 reassociation、公开 scalable-vector KIR/ABI、GPU target、cross-compilation、
runtime adaptive optimization、public JIT API、profile server/telemetry、任意 workload/value
recording，以及搜索式 Auto-Tuning。有界、可复现 offline Auto-Tuning 仍留到 0.14。
