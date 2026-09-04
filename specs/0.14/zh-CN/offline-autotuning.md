# CK 0.14 离线自动调优规范

[English](../offline-autotuning.md)

状态：CK 0.14.0 提议设计

已接纳基线修订：v0.13 修复候选 ee8dc5f25e3df085b359608c57a0fba0f3490213

本文档是 CK 0.14 实现的规范性依据，定义一个有界、可复现、可缓存的提前
编译自动调优系统。本文档不表示实现或者版本验收已经完成。

实现最初基于 v0.13 候选 `94aad2d6af8cea394ad2d2b311cf97fdb8bfbf05`。
最终验收前，已逐文件审计并以 v0.14 等价修复吸收该候选到最终接纳修订
`ee8dc5f25e3df085b359608c57a0fba0f3490213` 的累计提交差异。主动替代项记录在
实施期设计复诊 10；任何语义差异都不得通过适配测试来掩盖。

Linux 上继承的 schema-7 runtime sample 使用当前线程 CPU time 计量不变的 native
kernel-call loop；既有的单允许 CPU affinity scope 与每个保留七次计时 sample 前的一轮
`bounded-upper-band-v1` 校准仍为必需。这会排除
托管 runner 未调度 benchmark 线程的时间，但不会排除任何 kernel 工作；非 Linux 宿主继续使用
schema 7 定义的 monotonic timer。Historical schema-8 report 及其 evidence 必须在 retained
checker 执行前复制到可上传 replay bundle，使 checker 拒绝仍可诊断。

## 1. 目标

CK 0.14 应允许用户针对一组有代表性的工作负载，显式编译原生可执行文件或
动态库，并通过真实测量选择更快且合法的优化计划。

调优器应当：

- 使用 CK 静态分析构造规模较小、合法且高价值的候选集合；
- 使用用户提供的、可重复执行的工作负载驱动程序测量候选；
- 使用与搜索集不同的验证用例验证获选计划；
- 在可移植的决策文件中记录全部输入、决策、测量数据和拒绝原因；
- 允许后续构建精确重放获选计划，而不再运行驱动程序；
- 保持 CK 安全语义、确定性编译以及最终产物现有的自包含系统运行时策略。

性能目标不是“碰巧由 LLVM 优化成功”。事实、决策空间、合法性检查、测量
策略和可复现重放均由 CK 掌握；LLVM 继续作为原生代码生成器。

## 2. 产品决策

CK 0.14 采用有界的离线两阶段自动调优：

1. 静态分析与 CK 成本模型对确定性的合法前沿进行排序。
2. 用户提供的搜索工作负载测量一组有界的最终候选。
3. 独立的验证工作负载对领先计划完整验证两次。
4. 测量收益证书授权一个精确的优化计划。
5. 普通的提前编译构建可以从决策文件重放该计划。

这是显式选择加入的工作流。普通 check、run、build 和 release 命令不得启动
调优、执行驱动程序或者隐式使用调优决策。

### 2.1 未选择的替代方案

- 普通构建继续使用纯静态选择，但它从未观察用户工作负载，因此不能称为
  自动调优。
- 拒绝穷举或随机实测搜索，因为其边界与可复现性不可预测。
- 延后 Bayesian 和在线学习搜索，因为 Schema 1 要求封闭、可审计且没有训练
  状态的算法。
- 拒绝搜索 LLVM Pass 流水线或后端参数，因为 CK 无法独立描述并验证该语义
  决策空间。
- 拒绝自适应 JIT 调优，因为它会增加预热、运行时机制和不确定性，并产生与
  提前编译产物不同的部署契约。
- 拒绝隐式生成 Profile 或执行工作负载，因为构建源码不得意外运行用户程序。

## 3. 范围

### 3.1 包含

- 本机原生提前编译。
- O3 优化级别。
- 由 cpu=native 选出的精确本机 CPU 和特性集合。
- 可执行文件和动态库输出。
- 能表示为有限备选项的现有 CK 优化决策。
- 可选使用由完全相同 v0.14 编译器/源码/Schema 身份采集的 CK Profile，作为
  候选排序和代码生成输入。缓存身份改为 Schema 5 后，不接受 v0.13 Profile。
- 确定性的候选生成、测量调度、选择、缓存、检查和重放。
- 稳定的文本和 JSON 检查输出。

### 3.2 不包含

- 自适应 JIT 编译和 ORC 运行时调优。
- 在普通 ckc run 或 ckc build 中隐式调优。
- 工作负载合成、模糊测试生成的工作负载，或者由 profile 虚构用例。
- 静态库和目标文件调优。
- 可移植基线或者交叉编译调优。
- 使用一个调优决策覆盖多个 CPU 变体或者机器集群。
- 分布式或远程调优服务。
- 任意 LLVM 参数、Pass 流水线或者插件搜索。
- 间接调用提升、可扩展 KIR、源码级 SIMD、宽松浮点、GPU 卸载和新的
  源语言语法。
- 公共 ABI、原生 ABI、运行时 ABI 或运行时依赖变更。

这些排除项是有意为之。它们可以成为后续版本的候选，但不得作为实现细节
被悄悄带入 CK 0.14。

## 4. 前置条件与兼容性

只有同时满足以下条件，调优才有效：

- 输出类型为 executable 或 dynamic；
- 选择原生后端；
- 优化级别为 O3；
- 目标为当前宿主目标；
- 选择 cpu=native；
- 未选择 cpu=multiversion；
- 禁用 Profile 生成，只接受显式 Profile-use 输入；
- 禁用契约消毒器模式；
- 每个可选 profile 和编译模式都已显式给出且有效；
- 工作负载清单和每个声明输入都通过校验。

现有 safe、strict、checked、unchecked、contract、overflow、浮点、PGO 和
multiversion 语义继续具有最高约束力。调优只能从严格保持所选模式的计划中
选择，不得弱化保护检查，也不得改变可观察失败次序。

C 和 WebAssembly 后端不受影响。源语言和公共 ABI 均不变。

tune build 绝不代替用户执行 Profile 生成或“训练”。使用 pgo-use 时，所指定
Profile 是不可变输入。源码变更后重新采集 Profile，仍是单独的显式工作流。

## 5. 术语

- 基线：相同源码、模式、目标、输出类型和可选 profile 下，不带调优覆盖的、
  精确的 v0.13 风格普通 O3 原生产物。
- 决策点：CK 拥有一组有限合法优化备选项的稳定编译器位置。
- 调优单元：共享循环根、辅助函数、专用化边界或者代码体积交互的决策点的
  确定性聚类。
- 计划：所有调优单元中非基线选择的规范化集合。
- 试验产物：仅用于测量、不可发布的临时产物。
- 搜索用例：用于比较基线和最终候选的清单工作负载。
- 验证用例：仅在搜索排序后使用、且与搜索用例不同的清单工作负载。
- 决策文件：包含身份、测量以及获选计划或选择基线原因的二进制 .cktune
  记录。
- 测量收益证书：证明一个精确合法计划通过固定验证阈值的证据。

## 6. 命令行契约

### 6.1 调优并构建

主命令为：

    ckc tune build <file> --config <workload.cktune.toml>
      --out <artifact> --kind <executable|dynamic>
      --cpu native -O3
      [--target <host-triple>]
      [--pgo-use <profile.ckprof>]
      [现有语义与代码生成模式]
      [--budget <quick|standard|thorough>]
      [--tune-out <decision.cktune>]
      [--no-tune-cache]
      [--explain-optimization]

默认预算为 standard。现有 `NativeArtifactPaths` 解析定义完整输出集：可执行
文件只有主输出；动态库包括主库和生成的 C 头文件；Windows 动态库还包括
Import Library。省略 `--tune-out` 时，决策路径为解析后主输出路径追加
`.cktune`。显式决策路径必须与所有输出具有相同的规范父目录。提供 target
时必须规范化为精确宿主三元组；省略时选择同一三元组。

对 `tune build`，`--explain-optimization` 保留普通诊断，并为每个获选
predicated-update alternative 输出由独立 checker 验证、且在
[`predicated-update-performance-1.md`](../predicated-update-performance-1.md) 中定义的
规范 attestation。它不改变 candidate discovery、selection、decision bytes 或 artifact bytes。

所有目标经过规范目标解析后必须彼此不同，也不得与源码、Manifest、驱动、
Profile、声明输入或者另一目标形成别名。Schema 1 不支持跨目录输出集。决策
记录存在的主输出、头文件和 Import Library 的角色、规范逻辑名称、暂存字节
摘要与物理体积。

每个调优目标叶名为 1..255 个 ASCII 字节，匹配
`[A-Za-z0-9][A-Za-z0-9._-]*`，不是 `.`/`..`，不以 `.ckc-tune-` 开头，
也不是 Windows 设备名或以点/空格结尾的名称。该语法排除 `~`，所以请求的目标
不能拼写新增长名通常自动生成的 Windows 8.3 形式，但该事实不用于处理已有目录项。
在 Windows 上，CK 按 Handle 打开每个已有目标，取得权威长叶名和任何短叶名，
把别名拼写替换为长叶名后再构造规范路径和 Key；查询不受支持、结果不一致或与
另一目标碰撞时失败关闭。因此手工指定的 `ALT.DLL` 一类短名不能取得独立锁。
CK 不跟随链接地打开已经存在的共同父目录，
并从平台适配器取得该精确目录的稳定卷/目录身份和查找大小写行为。
适配器必须区分大小写敏感与 ASCII 大小写不敏感；未知、可变或不支持的等价性
在暂存前失败。ASCII 限制排除 Unicode 规范化别名。全部别名检查、排序键和锁
均使用父目录身份加规范长叶名的精确或 ASCII 小写查找键；已有目标还要按 Handle
身份复核。规范化时不存在的目标，在取得完整锁集合后、开始暂存前，按相同的
no-follow 长短名流程重新检查；命名空间变化会释放锁并重新开始。CK 从不创建或
指定短名。

发布为每个规范决策、产物或 Sidecar 目标使用一个持久同目录锁，并使用集合
Journal、Stage 与 Backup 文件。目标锁名称取
`H("CK-TUNE-DESTINATION\0", DestinationKeyMaterial)` 全部 64 个十六进制字符；
集合 Journal 名称同样使用 `H("CK-TUNE-OUTPUT-SET\0", OutputSetMaterial)` 的
全部 64 个字符。CK 打开任何保留文件时不得跟随符号链接或 Reparse Point，按规范目标 id
顺序取得完整重叠闭包中的全部目标锁，并在恢复与发布全程持有。锁文件与 Journal
均存储并校验完整 32 字节 id；身份不匹配属于硬错误，绝不视为同一对象。因此即使
两个命令为同一主产物选择了不同显式决策路径，它们仍会在主目标锁上串行化。

英中设计共用的规范附件
[`publication-journal-1.md`](../publication-journal-1.md) 是唯一字节、文件名、
Barrier、阶段与恢复级权威；下述概览不能弱化或重排该协议。

有界 Journal Schema 1 包含事务 id、输出集 id、阶段和持久恢复方向；按发布顺序 decision、
header、import-library、primary，对每个存在的目标，
还包含目标、Stage 与 Backup 的 Basename、旧文件存在位、旧摘要、新摘要与新
体积。阶段为 `Prepared=1`、`BackedUp=2`、`DecisionPublished=3`、
`SidecarsPublished=4`、`PrimaryPublished=5`、`Committed=6`。发布严格为：

1. 创建并验证所有同目录 Stage 文件，Flush 每个文件，再 Flush 父目录；
2. 写入并 Flush `Prepared`，再 Flush 父目录；
3. 将各旧目标重命名为 Backup，Flush 父目录，再写入并 Flush `BackedUp`；
4. 原子重命名并 Flush 决策和父目录，再记录 `DecisionPublished`；
5. 如存在则依次发布并 Flush 头文件、Import Library，Flush 父目录，再记录
   `SidecarsPublished`；
6. 最后发布并 Flush 主产物和父目录，再记录 `PrimaryPublished`；
7. 验证整个新输出集，记录并 Flush `Committed`，删除 Backup、Stage 与
   Journal，再 Flush 目录。

目录 Flush 使用各平台有文档的最强等价机制，并由每个原生宿主恢复测试覆盖。
锁和 Journal 最终名称只在完整字节 Flush 后原子暴露。主产物发布前的错误先
持久切换方向再回滚；主产物发布时或之后则完成前滚。重启时先解析附件中完备的
Active/Update/私有写入状态表，再计算全部目标、Stage 与 Backup 摘要。持久回滚
方向永远继续回滚；否则由阶段与主产物身份选择规定的前滚或持久回滚路径。两者都必须幂等。无法解释或未
记录的摘要组合是硬恢复错误，必须保留
Journal 与全部证据，不得猜测；恢复成功前后续命令不得接触该集合。

本规范中的事务输出集发布特指这种带 Journal、主输出最后发布的协议，不宣称
多个文件同时原子可见。成对消费者仅在决策以及全部记录角色都与磁盘摘要和
体积一致后接受发布结果。

如果全部必要基线与存续候选测量有效且稳定，但没有候选满足固定收益阈值，
命令成功输出基线产物和一份选择基线的决策文件。已经被规范超时规则移除的
候选，不会导致其余测量流变得不完整。

配置、身份、编译、正确性、驱动程序、协议、基线超时、进程控制、不稳定
或者验证未完成均属于错误，不得产生新产物或决策输出。第 8 节严格定义的
校准后候选超时规则是唯一的超时例外。

### 6.2 检查

    ckc tune inspect <decision.cktune> [--json]

检查为只读操作，不要求原始源码、驱动程序、profile 或缓存存在。展示内容
前必须完整校验该有界文件。

### 6.3 重放

    ckc build <file> --out <artifact> --kind <executable|dynamic>
      --cpu native -O3 --tune-use <decision.cktune>
      [调优时使用的相同 profile 和模式]

显式 tune-use 采用失败关闭策略。源码、编译器、Schema、目标、原生 CPU、
特性、profile、模式、输出类型、候选前沿或者计划有任何不匹配，均为硬错误，
不得静默退回普通优化。

CK 0.14 不为 run 或 emit-kir 增加 tune-use。精确的获选计划通过
ckc tune inspect 检查。

### 6.4 普通命令

未使用 tune build 或 tune-use 时：

- 不启动工作负载进程；
- 不读写调优缓存；
- 不允许调优决策改变优化器行为；
- 普通优化器的阈值和决策保持不变，但允许 CK 0.14 所必需的版本化内部
  Schema 和诊断维护。

## 7. 工作负载清单

### 7.1 格式

输入是 UTF-8 TOML 文件，约定命名为 workload.cktune.toml。Schema 1 是封闭
Schema：未知、重复、缺失、类型错误或者超出范围的字段均为错误。

清单声明：

- schema = 1；
- 一个只用于取得不可变快照的宿主原生驱动程序可执行路径；
- 一个操作性输入根目录，默认是 Manifest 目录；
- 不经过 shell 的固定 argv 向量；
- 可选的、显式允许继承的环境变量；
- 要计算摘要的驱动程序输入文件列表；
- 一至十六个带权用例；
- 每个用例的稳定标识、角色、种子、权重和期望摘要；
- Schema 范围内的单次调用超时；I/O 限制由本规范固定，总墙钟由所选预算
  预设固定。

必须至少包含一个搜索用例和一个验证用例。两个分区中的用例标识和种子必须
各不相同。标识只能使用 ASCII 字母、数字、下划线、短横线和点，长度不超过
64 字节。权重为正 u32 值。

Schema 1 只包含以下字段：

| TOML 位置 | 字段 | 类型与规则 |
| --- | --- | --- |
| 根 | schema | 必需整数，恰好为 1 |
| runner | path | 指向宿主格式原生可执行文件的必需 UTF-8 路径；相对值从规范 Manifest 父目录解析，允许绝对值；它属于操作路径，不属于规范身份 |
| runner | input_root | 从 Manifest 目录解析的可选 UTF-8 目录路径，默认 `.`；仅属于操作路径 |
| runner | args | 最多 64 个 `Text` 字符串的可选数组，默认空；每项必须已经 NFC、无 NUL、最多 4,096 个 UTF-8 字节，总计最多 64 KiB |
| runner | inputs | 最多 64 个相对输入根的普通文件 `Text` 路径，默认空；每个路径必须已经 NFC、无 NUL、非绝对、无父级穿越且最多 4,096 个 UTF-8 字节；每个文件最多 1 GiB，总计最多 4 GiB |
| runner | inherit_env | 最多 16 个匹配 [A-Za-z_][A-Za-z0-9_]* 的唯一名称，默认空 |
| runner | timeout_ms | 100 至 120,000 的可选整数，默认 30,000 |
| 每个 case 条目 | id | 符合上述语法和长度的必需唯一标识 |
| 每个 case 条目 | role | 必需字符串，只能是 search 或 validation |
| 每个 case 条目 | seed | 必需 u64 整数 |
| 每个 case 条目 | weight | 必需正 u32 整数 |
| 每个 case 条目 | expected_digest | 必需的 64 字符小写十六进制摘要 |

规范示例为：

    schema = 1

    [runner]
    path = "./build/tune-harness"
    input_root = "."
    args = ["--ck-tune"]
    inputs = ["data/search.bin", "data/validation.bin"]
    inherit_env = []
    timeout_ms = 30000

    [[case]]
    id = "search-medium"
    role = "search"
    seed = 101
    weight = 2
    expected_digest = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

    [[case]]
    id = "validation-medium"
    role = "validation"
    seed = 202
    weight = 2
    expected_digest = "abcdef0123456789abcdef0123456789abcdef0123456789abcdef0123456789"

Unix 驱动程序接收通过校验的精确 UTF-8 argv 字节。Windows 把 runner 快照路径
另行作为 `lpApplicationName` 传入并使用同一路径作为 argv 第 0 项；每个已接受
Unicode 标量序列无规范化、无损地转为 UTF-16，应用 Microsoft UCRT argv 逆算法，
并总是为每个参数加引号：每段
`n` 个反斜线在普通文字前输出 `n` 个，在引号前输出 `2n+1` 个再输出引号，在
结束引号前输出 `2n` 个；参数间以一个 U+0020 连接。符合规范的 runner 使用
UCRT 兼容 argv 解码；自行以不同规则解析原始命令行者不属于 Schema 1。Golden
进程测试执行保留的探针，对空串、空白、引号、尾反斜线和非 ASCII 参数逐项要求
还原精确 argv。
CK 不得在校验后再次规范化参数。
驱动程序工作目录始终为 CK_TUNE_TEMP，清单不能覆盖它。资源输出限制由本规范
固定，不是可配置的清单字段。

### 7.2 路径与输入

绝对 `runner.path` 按原值使用；相对值（包括父级分量）从规范 Manifest 父目录
解析，绝不相对调用方或源码工作目录。CK 逐组件遍历且不跟随符号链接/Reparse
Point，在快照前按 Handle 打开最终文件；任何歧义都报错。`input_root` 从规范 Manifest 目录解析，并以一个稳定的 no-follow 目录 Handle
打开；它的拼写可以包含父级分量，但解析后的根在打开输入前固定。每个逻辑输入
路径只能在该 Handle 下解析；绝对路径、逻辑路径内部父级穿越、符号链接/Reparse
歧义、非普通文件以及逃逸根目录都必须拒绝。解析到同一文件 Handle 身份的两个
输入也必须拒绝。驱动程序可以位于
该目录外，但必须由显式路径指定，禁止通过 PATH 查找。Schema 1 只接受宿主
原生 ELF、Mach-O 或 PE/COFF 可执行格式，不接受脚本或解释器指令。

规范 Manifest 身份不是 TOML 原始字节流，而是使用规范决策 Schema 附件的
Primitive Framing，按以下顺序计算
`H("CK-TUNE-MANIFEST\0", ManifestMaterial)`：

1. schema；
2. runner argv 字符串；
3. 排序后的有效继承环境变量名称/值长度/值摘要记录；
4. timeout；
5. 按 Manifest 顺序排列的 `ManifestInputMaterial`，Tag 1..3 依次包含逻辑
   路径、内容摘要和字节长度；
6. 按规范标识排序的 `ManifestCaseMaterial`，Tag 1..5 依次包含 case id、role、
   seed、weight 和期望摘要；
7. 不可变 runner 快照字节长度与内容摘要。

精确 Material 记录及外层 Tag 只在
[`decision-schema-1.md`](../decision-schema-1.md) 定义一次；此列表只是对齐
概览。`input_root` 与源路径拼写只属于操作信息并被排除；逻辑路径和不可变内容
仍进入规范身份。

操作 Manifest 与 runner 绝对路径、TOML 空白/注释、时间戳、除可执行有效性
外的文件权限以及临时路径均被排除。任何逻辑字段或内容字节变化都会使复用
失效。

会话开始时，CK 在不跟随符号链接或 Reparse Point 的前提下打开 runner 和
每个已验证输入。CK 将 runner 复制到私有会话快照，验证复制后字节与宿主
可执行格式，并在整个会话中只执行该快照。每个输入同样被流式复制到私有
不可变内容快照并校验摘要。每次计时调用前，CK 将输入快照复制到
`CK_TUNE_TEMP/inputs` 下的扁平内容寻址文件，名称严格由零基 Manifest 序号的
八个小写十六进制数字、一个 `-`、64 字符小写内容摘要和 `.bin` 组成，并创建
只读 `CK_TUNE_INPUT_MAP`。该有界规范 Map 严格由八个 ASCII 字节 `CKTIMAP1`、
`U32_BE(input_count)` 以及每个 Manifest 顺序输入的一条连续记录组成。记录依次为
逻辑路径 `Text`、暂存 ASCII Basename `Text`、字节数 `U64` 与摘要 `D32`；`Text`
是 `U32_BE(UTF-8 字节数)` 后接对应字节，`U64` 为大端，`D32` 恰为 32 字节。
数量范围为 0..64，解析必须恰好结束于最后一条记录；截断、溢出、无效 UTF-8、
数量不符或尾随字节均报错。生成的长名在精确和 ASCII 折叠比较下均唯一。
CK 打开实际临时父目录并将每项 create-new/no-follow；Windows 还枚举全部生成项的
长短名对并要求一一对应，任何长名或短名都不得等于或解析到另一暂存项；不支持
或不一致的枚举会失败关闭。全部文件复算摘要。输入准备不计入测量区间，每次
调用收到全新 Map 与文件，因此一次调用不能改变后续输入。

### 7.3 环境

驱动程序从空环境启动。在 Windows 上，CK 只能提供创建进程所需的最小
SystemRoot 和 WINDIR 值。其他继承变量必须显式加入允许列表；请求的变量
不存在即报错。Unix 名称按字节唯一，Windows 名称按 ASCII 大小写不敏感比较
唯一；重复或非规范拼写均报错。上限 16 约束完整有效环境，而不仅是用户允许
列表。Windows 先插入所需基础名称；规范拼写的同名允许项引用已有同一记录，
大小写冲突则拒绝；Union 超过 16 时校验失败。每个值最多 4,096 字节，完整有效环境最多
65,536 字节；NUL 必须拒绝。包括 Windows 平台基础值在内的每个有效名称
和值身份都进入调优身份。

Unix 规范继承值是精确非 NUL 字节；Windows 值以不做规范化的 UTF-8 编码，
无法表示的值必须拒绝。
公共决策只记录名称、精确字节长度以及
`H("CK-TUNE-ENV-VALUE\0", name Text, value Bytes)`；实际值只存在于私有会话
内存和进程状态中，inspect 绝不渲染它。

CK 设置以下协议变量：

    CK_TUNE_PROTOCOL=1
    CK_TUNE_ARTIFACT=<候选产物绝对路径>
    CK_TUNE_ARTIFACT_KIND=<executable|dynamic>
    CK_TUNE_CASE=<用例标识>
    CK_TUNE_SEED=<无符号十进制 u64>
    CK_TUNE_ITERATIONS=<无符号十进制 u64>
    CK_TUNE_TEMP=<每次运行的私有目录绝对路径>
    CK_TUNE_INPUT_MAP=<私有输入 Map 绝对路径>

argv 和环境直接传给进程创建接口。CK 绝不拼接 shell 命令字符串。

### 7.4 驱动程序责任

驱动程序仅用于调优，不链接到最终产物中，最终产物运行时也不依赖它。驱动
程序必须从 CK_TUNE_INPUT_MAP 读取声明输入位置，加载或执行
CK_TUNE_ARTIFACT，运行恰好 CK_TUNE_ITERATIONS 次指定
用例逻辑迭代，并生成确定性的正确性摘要。

对于动态库，驱动程序负责加载并调用其导出 ABI；对于可执行文件，驱动程序
负责调用或者驱动它。Artifact kind 只来自 ckc tune build，并通过
CK_TUNE_ARTIFACT_KIND 传入，不是 Manifest 字段。CK 不推断应用协议。

驱动程序是用户明确授权的任意代码。CK 不宣称提供跨平台的文件系统或网络
沙箱。如果驱动程序不可信，用户必须自行应用操作系统沙箱。

## 8. 驱动协议与计时

### 8.1 输出

成功的驱动程序以状态零退出，并向 stdout 精确写入一行：

    CKTUNE/1 <case-id> <seed-u64> <iterations-u64> <completed-u64> <digest>\n

digest 必须恰好是 64 个小写十六进制字符。completed 必须等于 iterations，
回显的用例、种子和迭代数必须与请求一致。任何额外 stdout 都是协议错误。

stdout 上限为 4 KiB。stderr 为诊断而捕获，上限为 1 MiB。截断、协议数据
不是有效 UTF-8、非零退出、信号终止或者格式错误均为错误。

### 8.2 正确性

每个用例和种子的期望摘要由清单声明。CK 接受校准前，基线必须匹配该摘要。
候选的每次调用，包括预热、校准确认、搜索采样和验证采样，也都必须返回
相同摘要。

摘要不匹配表示编译器正确性故障或者无效驱动程序，而不是候选速度较慢。
整个调优会话必须中止，不得将其转换为普通候选拒绝。

### 8.3 计时与进程控制

CK 使用高分辨率单调时钟在驱动程序外部测量经过时间。它为每次调用建立私有
目录，执行输出与超时限制，并清理临时文件。

Schema 1 提供协作式进程 Containment，而不是敌对代码沙箱：

- Windows 将 runner 以暂停状态创建，放入禁止 Breakaway 且设置
  KILL_ON_JOB_CLOSE 的 Job Object，然后恢复执行；无法建立该 Job 即调优错误。
- Linux 与 Darwin 在 runner 代码执行前创建新进程组。runner 契约禁止
  setsid、改变进程组、Double-fork Daemon、调用结束后仍存活的后台工作以及
  任何等价逃逸。
- 超时或输出溢出时，CK 先请求 Group/Job 终止并等待 250 ms，然后强制终止，
  回收直接 runner，并最多再等待 2,000 ms 让协作式 Containment 变空。
- 无法建立 Containment、终止、回收 runner 或观察到协作式 Containment
  为空，都会中止会话。

故意逃逸的 POSIX 后代属于任意敌对同用户行为，位于“不提供沙箱”的契约边界
外。CK 不声称能够发现或终止这种进程。

驱动程序必须批量执行足够多的工作，以摊薄启动成本。对按规范标识排序的每个
搜索与验证用例，基线校准为：

1. 从 iterations = 1 开始；
2. 最多执行 32 次计时基线尝试；
3. 每次尝试后都验证期望摘要；
4. 接受第一次持续至少 50 ms 的尝试；
5. 否则以经过检查的 u64 算术将 iterations 翻倍后重试；
6. 接受后，以相同 iterations 和摘要再执行一次基线确认调用；
7. 接受的尝试超过 250 ms 时记录 calibrationOvershoot。

32 次尝试后仍未达到 50 ms、算术溢出、基线超时或确认失败都会中止会话。
250 ms 是首选上限，不是拒绝单次逻辑迭代本身较粗的工作负载的理由。接受的
迭代数在该用例的搜索与两轮验证中保持固定，验证绝不重新校准。

成功完成基线校准后，候选耗尽完整配置超时即为规范性能拒绝。CK 执行上述
Containment 关闭、记录超时，并在全部后续行和轮次中跳过该不可变通道槽位，
但不改变存续通道顺序。对该已拒绝候选，这视为验证已经完成。基线超时、
缩短的 Deadline、崩溃、协议错误、正确性摘要不匹配或 Containment 故障都会
中止会话。

### 8.4 调用状态机与顺序

全部用例校准后，固定状态机为：

1. 完成产物体积拒绝和编译后最终候选选择后，每个体积合法的测量最终候选按
   计划摘要排序，对每个按 case-id 排序的搜索用例执行一次正确性冒烟调用；
2. 搜索在不可变的“基线加最终候选”通道列表上执行三行预热和二十行测量；
3. 排名最好的有界存续候选进入验证；
4. 验证第 1 轮在不可变的“基线加入围者”列表上执行三行预热和二十行测量；
5. 验证第 2 轮以不同顺序域重复同一矩阵；
6. 只有全部必要存续流完整后才执行选择。

候选冒烟前，CK 派生：

    session_digest = H("CK-TUNE-SESSION\0",
                       Identity,
                       Contract,
                       Workload,
                       Environment Tag 1..16,
                       完整 Frontier,
                       基线计划/目标图/链接配方/体积 Tuple)

`H` 与每个规范记录由 `decision-schema-1.md` 定义。校准记录、测量、正确性
结果、缓存来源、临时路径、时间戳和发布目标均排除。派生摘要存为 Environment
Tag 18，是唯一测量顺序种子。

一次预热通道评估恰好调用一次。一次测量通道评估恰好调用三次并存储最小值。
每次调用都验证协议与正确性。基线存在于每个阶段。

冒烟使用 Phase 1、Round 0、Row 0、Call 1；候选与用例保持上述计划摘要和
case-id 顺序，因此冒烟无需通道轮转。

用例按 case-id 存储；通道依次为基线和按计划摘要升序排列的候选。每一行使用
附件的规范类型编码计算
`H("CK-TUNE-ORDER\0", sessionDigest D32, phase U8, round U8, row U32,
caseId Text)`。将前八个字节解释为大端 u64，对该用例的通道数取模得到左轮转
量，并将完整摘要存为该行 `permutationKey`。相同 Domain 和前四个类型值后接
`Bytes([0xff])`，得到用例列表轮转量。Phase 值依次为：
1 候选冒烟、2 搜索预热、3 搜索测量、4 验证一预热、5 验证一测量、
6 验证二预热、7 验证二测量。验证外 round 为零，验证内为 1 或 2。被拒绝的通道槽位继续
作为显式 skip，因此移除候选不能重排有利样本。

候选超时时丢弃未完成流，但保留精确超时坐标。决策以规范顺序存储实际轮转
调用计划中、在该坐标之前已经完成第二十行的全部流集合；该集合不要求是规范
流顺序的前缀。检查器从 Session Digest 和超时坐标重新计算此集合。

启动一次调用前，CK 要求会话墙钟至少还剩完整配置超时加固定 2,250 ms
Containment 清理余量；否则不启动进程，直接以证据不完整中止。CK 绝不为了
适配会话 Deadline 缩短 runner 超时。只有耗尽完整配置候选超时才是性能拒绝。

## 9. 合法候选模型

### 9.1 CK 拥有的备选项

CK 0.14 只能调优由 CK 事实推导且由 CK 拥有的有限备选项：

- 直接调用内联选择；
- 函数专用化以及带保护的值或长度专用化选择；
- 循环展开因子；
- Loop SIMD 向量宽度、交错因子和盈亏平衡阈值选择；
- SLP 打包选择；
- 短 slice 和循环版本化选择；
- CK 拥有的基本块、函数和 Section 布局备选项。

Loop SIMD 类还拥有针对规范循环形态
`if candidate < old { dst[index] = candidate }` 的 predicated same-place update
备选项。这不是 masked-memory 支持。合法改写从精确目标位置加载 `old`，计算向量
predicate，在 `candidate` 与 `old` 之间 select，再执行一次普通的 unmasked vector store。
只有不可变改写前 KIR 上同时闭合以下条件时才可使用：

- 一条分支路径恰好包含一次 store，空路径不含 memory operation 并直接进入
  merge/latch，store 路径也汇入同一个 merge/latch；
- store 目标与支配它的 `old` load 指向相同 affine place，消费其 incoming Memory SSA
  version，且二者之间不存在 memory definition；
- condition、candidate 与 index computation 不含 call、print、volatile access、possible
  failure 或其他 ordered effect；
- dependence proof 排除了全部 load 与获选 store 的 loop-carried 和 cross-lane conflict；
- strict floating-point compare/select 在不使用 fast math 或 contraction 的情况下保持
  unordered comparison、NaN、infinity、signed zero 与 operand order；
- checked mode 下每个 vector lane 都已证明在范围内，且每个 checked arithmetic operation
  都已证明不会失败。

独立 vector checker 必须重建 diamond、same-place relation、affine access、Memory SSA
version、lane bound、dependence result 以及精确 compare/select/store 改写。缺少任一证明都
拒绝候选，并保持 scalar loop 逐字节不变。接受的循环在需要时保留普通 runtime
alias/versioning guard，并始终保留有序 scalar epilogue。该备选项复用现有 Loop SIMD unit
class 及其 vector-width、interleave-factor（UF）和 break-even 参数，不增加 decision-file tag、
语言构造、public/native/runtime ABI 表面或 target feature。
对每个合格循环，proposer 生成该 payload 已能表示且 target 支持的、规范有界的不同 VF/UF
组合，而不是提前折叠为 ordinary cost-model winner。Ordinary winner 继续作为 baseline；合法
组合进入既有 unit-variant 上限，并由不变的 deterministic frontier search 测量。任何 target
不支持的 vector width、operation 或 feature 都不得作为 trial 发出。

每个决策点和备选项都具有规范、稳定的标识、前置条件摘要和顺序。追踪记录
同时记录接受和拒绝的备选项。

规范调优前快照是经过验证的 v0.13 O3 KIR：已完成 CFG 规范化、初始
SCCP/范围分析、循环规范化和第一次必要检查消除，且正好位于专用化之前。
候选重放的收益控制阶段顺序固定为：专用化、内联、短 slice/版本化、Loop
SIMD、展开、SLP、布局。调优单元按 `(阶段, unit id)` 排序，单元内备选项按
`(site id, alternative id)` 排序。既有必要分析、合法性检查、证明刷新与清理
Pass 保留在 v0.13 的阶段位置，计划不得选择或抑制它们。布局选择在原生 Lowering
前作为规范 KIR 元数据存在；后端仅在不变的固定 LLVM O3 流水线之后、目标文件
发射前消费它。空计划采用未经改变的 v0.13 O3 收益决策，不改变普通编译和既有
仅用于 O2 的 Late Layout 行为。
一旦存在非布局调优选择，重放就不得重新进入普通收益控制阶段：只执行精确选定的
备选项，随后执行必要分析与清理后缀。这保证 early-only 计划不会额外获得未记录的
普通专用化或内联选择。
布局只是元数据而不是 O3 改写，因此仅包含布局的计划必须先完成固定的普通 KIR
O3 后缀。随后 CK 把调优前选定的基本块排列投影到仍然存在的基本块 ID，并把固定
后缀新建的基本块按其后缀后的规范顺序追加。空投影或与规范顺序相同的投影是可测量
的无操作。该确定性投影属于 source-aware replay；布局不得抑制 KIR O3 后缀，也不得
引用已经不存在的基本块。
LLVM O3 可能在 Late Layout 前合法删除所选函数或基本块。因此后端在可行时保留
所选函数，随后依据 O3 后模块复核布局列表：完整应用仍然存在的映射；若不存在
完整的所选映射，则该布局成为可测量的无操作，而不是引用不存在的对象或改变
LLVM 流水线。

### 9.2 永不可调优

调优器不得改变：

- 语言或安全语义；
- 边界、溢出、契约或其他必要保护；
- 已证明事实、别名类别、指针来源、范围、对齐或副作用；
- 严格浮点行为；
- 失败和副作用次序；
- 源码 ABI 或公共 ABI；
- 目标三元组、CPU、特性集合或运行时 ABI；
- 消毒器模式；
- LLVM Pass 流水线或者任意后端参数。

测量只证明收益，不证明安全性、语义等价性或者目标合法性。

### 9.3 调优单元

在同一备选类别内，相互重叠的根、克隆辅助函数、专用化边界和共享代码体积
效应必须确定性聚类为一个调优单元。Schema 1 的 `Unit.class` 字段保证每个单元
类别单一；跨类别交互由跨多个单元的规范完整计划展开表达。若后置类别使先前
类别的锚点或前置条件失效，该展开保留为 `illegal` 搜索结果，搜索继续而不是
中止整个会话。一个会话最多考虑 64 个单元。超过上限的单元使用普通优化器
决策，并按规范排名而不是发现顺序选定。

每个单元最多暴露四个一致的非基线单元 Variant。一个单元 Variant 是一个或
多个决策点上的一组封闭选择；命令行绝不对独立决策点备选项构造笛卡尔积。
一个会话最多记录 4,096 个决策点、每计划 64 个非基线选择、256 个单元
Variant 和 16,384 次计划展开尝试。

### 9.4 试验类型状态

候选物化将合法性与静态收益判断分开：

1. CK 重新计算全部结构、证明、副作用、保护、失败次序、目标特性和增长检查。
2. 即使普通静态收益阈值拒绝某个备选项，CK 也必须把合法的测量专属备选项暴露给
   调优；该试验只能越过这个阈值，绝不能越过合法性、证明、目标、事务或增长检查。
3. 生成的试验产物具有不可发布类型状态。
4. 试验产物不得进入生产输出或生产目标缓存。
5. 只有有效测量收益证书才能授权在可发布构建中重放该精确计划。
6. 最终检查器在发布前独立重算计划合法性和测量阈值。

普通优化器阈值不变。CK 已判定计划合法后，如果候选仍构建失败，则属于
编译器错误，而不是搜索拒绝。

## 10. 确定性搜索

CK 在稳定调优单元和规范单元 Variant 集合上使用确定性 Beam Search。编译前
候选严格依次按以下键排序：

1. 以规范成本模型单位表示的预测动态成本；
2. 以规范成本模型单位表示的预测静态成本；
3. 规范 `print_kir_module` 字节长度；
4. 非基线选择数量；
5. 按单元顺序排列的备选类别枚举向量；
6. 按单元顺序排列的 `(unit id, variant id)` 字节对向量；
7. 计划摘要。

这些都是完整计划键，绝不把各 Variant 估计简单求和。每次合法扩展后，CK 在
同一调优前 KIR 的全新副本上重新应用完整计划，对结果完整 Module 运行规范成本
模型，并以经过检查的 `u64` 转换计算打印后规范 KIR 字节长度。失败或溢出属于
编译器错误。展开追踪记录全部三个完整计划指标。

编译前尚无产物字节数，因此它绝不参与该排名。编译后选择测量最终候选时，
以实际产物字节数替代规范 KIR 字节长度，其余排序键不变。

封闭算法为：

    beam = [baseline]
    expansions = 0
    for unit in canonical_unit_order:
        pool = beam                         # 免费携带基线
        for plan in beam_precompile_rank_order:
            for variant in unit.nonbaseline_variants_in_canonical_order:
                if expansions == expansion_limit: 停止全部后续展开
                ordinal = expansions
                expansions += 1
                派生 plan + variant，并运行全部 KIR 合法性/增长检查
                以 ordinal 记录尝试，包括非法、重复或者增长超限
                将合法且唯一的派生计划加入 pool
        unique = deduplicate(pool without baseline)
        beam = [baseline] + diversity_truncate(unique, beam_width)
    frontier = beam without baseline
    compile_selection = diversity_truncate(frontier, compile_attempt_limit)

携带基线不消耗 Beam 槽、展开或编译尝试。每次非基线派生在校验前消耗一次展开；
非法、重复、增长超限或 Cache Hit 都不返还。达到展开上限时，在下一次派生前
停止；已经接受的计划仍可参与，后续单元保持基线行为。一个计划被选中编译即
消耗一个编译尝试槽，即使已验证的编译 Cache Hit 避免了物理编译。CK 已声明
合法的计划若编译失败，属于编译器错误。

展开序号从零开始且连续：第一次记录为 0，之后每项严格加一，列表恰为
`0..expansions-1`，且 `expansions` 等于其长度。追踪必须包含上述嵌套循环在单元
耗尽或达到预设上限前产生的每次尝试；遗漏、插入、重排或错误分类均无效。

`deduplicate` 对同一计划摘要保留规范编译前排名最先出现者。
`diversity_truncate` 按固定类别顺序处理：内联、专用化、展开、Loop SIMD、
SLP、短 Slice/版本化、布局。有槽位时，每类取“最新非基线选择”属于该类的最佳
计划一次；若 Beam 比可用类别数更窄，则固定类别顺序优先。余下槽按全局排名
填充并跳过已选计划。Beam、编译选择和编译后最终候选选择使用同一规则，避免
静态模型淘汰全部结构不同的合法候选。

封闭预设如下：

| 预设 | Beam | 计划展开 | 候选编译尝试 | 测量最终候选 | 验证入围者 | 墙钟上限 |
| --- | ---: | ---: | ---: | ---: | ---: | ---: |
| quick | 4 | 1,024 | 8 | 4 | 2 | 600 s |
| standard | 8 | 4,096 | 16 | 8 | 3 | 1,800 s |
| thorough | 16 | 16,384 | 32 | 16 | 4 | 7,200 s |

基线始终存在且不计入候选编译尝试。Schema 1 不支持用户任意增大这些数值。

发生 Cache Miss 时，预设墙钟在 Manifest 校验与输入快照完成后、基线构造前
立即开始。它包括基线与候选编译、驱动准备、搜索、两轮验证、最终重放和事务
暂存。仅当剩余预算足以覆盖完整配置超时以及第 8.4 节固定的 2,250 ms 收容
余量时，才启动一次驱动调用；不得为适配会话预算缩短超时。精确完整决策的
Cache Hit 不启动搜索会话。

候选产物体积不得超过匹配基线字节数的 110%。现有 KIR、重写、专用化和
每 Pass 增长限制继续生效。被拒绝或无效的尝试消耗其记录的展开或编译预算，
绝不返还。在体积合法的已编译计划中，编译后排名至多选择预设的测量最终候选
数量；合法计划较少时，只以更小但完整的前沿继续。

只有完整确定性展开追踪和派生编译选择全部完成后，才可生成成功决策。最终检查器
从候选空间和预设重放封闭算法：决策 Trial 集必须恰好等于完整编译选择，按计划
摘要存储且不得遗漏或额外加入。检查器在隔离 Cache 中独立重建每个 Trial，并
核对计划、对象/链接身份、主产物摘要和实际字节数。体积拒绝项恰为超过 110% 的
Trial；对其余全部 Trial 以实际主产物字节替代 KIR 字节并应用同一 Diversity 规则，
得到精确测量最终候选集。每个体积合法非最终候选均为 `compiled-unmeasured`；每个
最终候选必须具有状态机规定的精确 Smoke/Search/Validation 结果和 Stream。墙钟
预算若导致展开或编译选择无法完成，命令失败且不写决策，绝不序列化部分前沿。

墙钟预算到期后，CK 停止创建候选。如果无法为所有必要入围者完成固定验证
协议，命令失败且不产生输出，不得从不完整证据中选择计划。

## 11. 测量与选择

### 11.1 搜索阶段

基线和参与测量的最终候选仅运行搜索用例。对每个用例：

- 校准固定迭代次数；
- 执行三行不计分的预热；
- 执行二十行测量；
- 每行按确定性轮转将每个活动通道恰好评估一次；
- 一次通道评估恰好包含三次相同驱动调用，其存储样本为三者最小值。

通道是基线或者一个候选。用例顺序、通道顺序和轮转由会话身份摘要导出，
不存在可变随机状态。

每个用例的时间是二十个存储样本的上中位数，即升序排列后从零开始编号的
第 10 个元素。只有至少 16 个样本位于该上中位数 80% 至 120% 的闭区间内，
该流才稳定。全部百分比比较使用经过检查的整数交叉相乘。任一必要基线、
候选或用例流不稳定都会导致错误。

搜索采用精确整数 Q32 归一化时间对候选排名：

    ratio_q32 = ceil(candidate_ns * 2^32 / baseline_ns)
    score_q32 = ceil(sum(weight * ratio_q32) / sum(weight))

选择过程不使用浮点运算。入围者严格依次按更低搜索得分、更小实际主产物
字节数、更少非基线选择、更低计划摘要形成全序；排名最好的有界入围者进入
验证。全部乘积与求和使用经过检查的 u128 算术，持久化 Q32 结果必须能装入
u64。

### 11.2 验证阶段

验证用例的标识和种子与所有搜索用例不同。每个入围者和基线使用相同固定
样本协议，进行两轮相互独立的完整验证。

每轮中，一个计划仅在满足以下全部条件时合格：

- 加权得分不高于基线的 0.97；
- 任一验证用例相对基线都不慢于 1.02；
- 二十行中至少有十六行的加权配对时间低于基线。

对配对行 r，CK 使用每个验证用例的第 r 行，以同聚合得分相同的逐用例
归一化公式计算加权 Q32 得分。“低于”表示严格低于基线 Q32 值 2^32。各验证
用例的行号保持同步。

Decision Schema 1 要求每个持久化轮次字段都由匹配校准记录和 Phase 5/7 原始流
校验推导：用例中位数、Q32 比率、加权聚合、稳定性、配对胜场、入围成员、阈值位
和排名都不得独立填写。

每轮内先对合格计划排名，平局依次按以下规则解决：

1. 验证得分更低；
2. 产物更小；
3. 非基线选择更少；
4. 计划摘要更小。

令 `Q1`、`Q2` 为第一轮和第二轮按排名排列的合格计划列表。选择严格按下述
互斥且完备的表执行：

| 条件 | 结果 |
| --- | --- |
| 不存在验证入围者 | 基线，`no-candidate` |
| `Q1` 或 `Q2` 为空 | 基线，`validation-threshold` |
| 两者非空且 `Q1[0] == Q2[0]` | 该计划，`tuned` |
| 两者非空且 `Q1[0] != Q2[0]` | 基线，`validation-disagreement` |

阈值结果下全部存续且完成验证的入围者结果均为 `validation-threshold`；分歧结果下
全部入围者均为 `validation-nonwinner`；调优结果下共同胜者为 `selected`，其余
入围者为 `validation-nonwinner`。no-candidate 行没有验证入围者结果，也不改写
此前的 Trial 结果。
超时入围者始终保留 `timed-out` 并从 `Q1`/`Q2` 排除，绝不被此表改写。

CK 观察到不利结果后不得自行决定重跑。因此，证据稳定但没有选出计划时，
生成一次成功的基线决策。

### 11.3 最终正确性规则

全部候选使用相同用例、种子、迭代次数、调度策略和正确性检查。搜索与验证
永不改变所选语言语义。调优器可以利用工作负载表达的领域约束，但只能通过
这样的优化计划实现：其保护和前置条件对每个合法程序输入都继续有效。

## 12. 决策文件

### 12.1 编码

公共决策文件包含：

- 由八字节 CKTUNE01 编码的魔数 CK TUNE 01；
- 格式 Schema 1；
- 契约 Schema 1；
- 测量 Schema 1；
- 检查 Schema 1；
- 计划 Schema 1；
- 规范的大端长度与计数；
- 规范的字段和集合顺序；
- 末尾经过域分离的 SHA-256 摘要。

外层编码沿用仓库现有规范 Profile Framing：

1. 第 0 至 7 字节为 CKTUNE01；
2. 第 8 至 11 字节为大端 u32 格式 Schema；
3. 每个字段依次为大端 u16 Tag、大端 u32 Payload 长度以及恰好对应数量的
   Payload 字节；
4. 最后 32 字节为
   SHA-256("CK-TUNING-DECISION\0" 后接此前全部文件字节)。

Schema 1 要求以下顶层 Tag 按升序且完整出现：

| Tag | Payload |
| ---: | --- |
| 1 | 编译器、源码、语义、Schema、目标、模式、Profile 与输出身份 |
| 2 | 冻结调优契约和全部数值策略常量 |
| 3 | Manifest、驱动程序、允许环境变量与声明输入身份 |
| 4 | 规范化测量环境与计时器证据 |
| 5 | 决策点、调优单元、备选项与候选前沿 |
| 6 | 基线和候选计划、产物、拒绝、正确性与原始测量 |
| 7 | 两轮验证、选择结果与测量收益证书 |
| 8 | 重放前沿、前后状态、获选代码身份、目标图、链接配方与缓存复用事实 |

嵌套记录使用相同的递增 u16 Tag/u32 长度 Framing。无符号标量使用固定宽度
大端编码；布尔值只能是一个规范字节 0 或 1；字符串是带长度的有效 UTF-8；
列表以经过检查的大端 u32 计数开头，并包含规范排序的记录；可选值使用显式
单字节存在判别值。每个有效决策只有一种编码。末尾 Hash 是重放与缓存键所用
的规范决策摘要。

文件最大 32 MiB，最多包含 33 个候选（含基线）、16 个用例，每个计划最多
64 个选择。以下上限具有规范性：

| 项目 | 上限 |
| --- | ---: |
| UTF-8 文本字段 / 诊断 | 4,096 字节 |
| argv 项数 / argv 总字节 | 64 / 65,536 |
| 环境变量项数 / 总字节 | 16 / 65,536 |
| 声明输入数 / 单输入 / 全部输入 | 64 / 1 GiB / 4 GiB |
| 用例 / 决策点 / 单元 / Variant | 16 / 4,096 / 64 / 256 |
| 每单元 Variant / 展开记录 | 4 / 16,384 |
| 候选（含基线）/ 每计划选择 | 33 / 64 |
| 输出记录 / 每完整流样本 | 3 / 20 |
| 测量流 | 1,584 |

未知、重复、截断、尾随、顺序错误、超限或者非规范内容都会被拒绝。只有在
边界和溢出检查完成后才允许分配内存。

### 12.2 封闭 Schema 1 记录

英中设计共用的规范附件 [`decision-schema-1.md`](../decision-schema-1.md) 是
唯一逐 Tag 线协议权威；它冻结全部 Primitive、嵌套 Tag、类型、枚举、必需/
可选状态、数值、边界与排序。下述内容仅是概览，不能覆盖该附件。必要字段恰好
出现一次，只有 `Opt<T>` 字段可选；全部 `Text` 字段禁止绝对路径。

公共 JSON 和稳定文本投影的精确契约由英中共用的规范附件
[`inspection-schema-1.md`](../inspection-schema-1.md) 单独冻结；Renderer 不得
发明、省略、本地化或者重排已经校验的决策数据。

顶层 Tag 1 `Identity` 包含：

| Tag | 类型 | 含义 |
| ---: | --- | --- |
| 1 | Text | CK 版本 |
| 2 | D32 | CK 源码身份 |
| 3 | Text | Rust 工具链身份 |
| 4 | Text | LLVM 身份 |
| 5 | D32 | LLVM Bridge 身份 |
| 6..15 | U32 | 语言、原生 ABI、运行时 ABI、KIR、证明、成本模型、目标、原生缓存、Profile、PGO 分析 Schema |
| 16 | D32 | 源码摘要 |
| 17 | D32 | 语义与契约摘要 |
| 18 | D32 | 调优前 KIR 摘要 |
| 19 | D32 | 编译模式摘要 |
| 20 | U8 | 输出类型枚举 |
| 21 | Record | 目标三元组、CPU、特性和目标 Profile 身份 |
| 22 | Opt<Record> | Profile Schema、编译器/源码身份、拓扑与字节摘要 |

顶层 Tag 2 `Contract` 包含五个 Schema 值、预算预设及其六个搜索边界、精确
产物比率、校准、采样、收容、稳定性与验证整数，以及域分离策略摘要。其 32 个
Tag 和精确值由附件固定，并必须等于第 8、10、11 节。

顶层 Tag 3 `Workload` 包含：

| Tag | 类型 | 含义 |
| ---: | --- | --- |
| 1 | D32 | 规范 Manifest 身份 |
| 2 | D32 | 私有驱动快照摘要 |
| 3 | U64 | 驱动快照长度 |
| 4 | List<Text> | 按 Manifest 顺序的 argv |
| 5 | List<Record> | 按平台规范变量名排序的有效环境 |
| 6 | U32 | 超时毫秒数 |
| 7 | List<Record> | 按 Manifest 顺序的输入：逻辑路径、摘要、体积 |
| 8 | List<Record> | 按用例 id 排序：id、角色枚举、种子、权重、期望摘要 |

顶层 Tag 4 `Environment` 包含封闭测量 Tuple、计时器与调度证据，以及按用例
id 排序的校准记录列表。每项校准记录迭代数、尝试数、接受与确认耗时以及
Overshoot，随后是派生 Session Digest 和本地测量缓存 Salt 摘要。不可得文本使用 `unavailable`；不可得
数字宿主事实使用显式 absent
可选状态。

顶层 Tag 5 `Frontier` 的 Tag 1 为候选空间摘要，Tag 2 为决策点，Tag 3 为
单元，Tag 4 为展开追踪。记录为：

| 记录 | 按 Tag 顺序的必要字段 |
| --- | --- |
| Site | 稳定 id `D32`；类别枚举 `U8`；根 id `D32`；前状态摘要 `D32`；规范排名 `U32`；稳定根 Anchor |
| Unit | 稳定单元 id `D32`；有序 Site-id 列表；基线状态摘要 `D32`；有序 Variant 列表 |
| UnitVariant | Variant id `D32`；类别枚举 `U8`；带封闭类别 Payload 的有序选择；孤立动态/静态/KIR 字节估计 `U64`；后状态摘要 `D32` |
| PlanChoice | 单元 id `D32`；Variant id `D32`；类别 `U8`；前状态 `D32`；后状态 `D32` |
| Expansion | 序号 `U32`；父计划 `D32`；单元 id `D32`；Variant id `D32`；处置枚举 `U8`；结果计划 `Opt<D32>`；诊断码 `U16`；三个可选完整计划排名指标 |

顶层 Tag 6 `Candidates` 的 Tag 1 为基线候选，Tag 2 是按计划摘要排序的非
基线候选列表。候选记录为：

| Tag | 类型 | 含义 |
| ---: | --- | --- |
| 1 | D32 | 计划摘要；基线使用规范空计划摘要 |
| 2 | List<PlanChoice> | 按单元顺序的选择 |
| 3 | D32 | 目标图摘要 |
| 4 | D32 | 链接配方摘要 |
| 5 | U64 | 实际主产物字节数 |
| 6 | U8 | 结果枚举 |
| 7 | U16 | 无诊断时为零的诊断码 |
| 8 | Opt<D32> | 正确性摘要 |
| 9 | List<Record> | 规范顺序的测量流 |
| 10 | Record | 不可变编译 CacheOrigin |
| 11 | Opt<Record> | 精确超时位置；仅 timed-out 结果要求存在 |
| 12 | D32 | 实际主产物内容摘要 |

测量流包含阶段、轮次、用例、计划、迭代数、二十个有序行记录和正确性摘要。
每行包含序号、排列键摘要、恰好三个原始纳秒调用以及其最小存储样本。预热
调用执行但不存储。规范超时候选不包含后续流，并携带精确超时位置；其他流
是否允许缺失只由附件的终态矩阵决定；矩阵要求的流不得缺失。

顶层 Tag 7 `Selection` 的 Tag 1、2 为第一、第二轮摘要；Tag 3 为获选计划
摘要；Tag 4 为选择原因枚举；Tag 5 为 `Opt<Certificate>`。每轮摘要包含用例
中位数、聚合 Q32 比率、稳定性、阈值结果与排名后入围计划摘要。证书包含精确
计划、前沿、策略、两轮、正确性、目标图与链接配方摘要。调优选择必须有证书；
基线选择禁止证书。

顶层 Tag 8 `Replay` 的 Tag 1..5 为前沿、获选前状态、获选后状态、目标图和
链接配方摘要；Tag 6 为按角色排序的输出记录；Tag 7..8 为不可变编译与测量
CacheOrigin 记录；Tag 9 为重放结果摘要；Tag 10 为与测量无关的选择身份摘要。
每个输出记录包含输出角色枚举、规范逻辑
Basename、暂存字节摘要和物理体积。可执行输出集仅含主输出；动态输出集还含
头文件，Windows 上再含 Import Library。

封闭枚举值为：

| 枚举 | 值 |
| --- | --- |
| 输出类型 | executable=1, dynamic=2 |
| 预算 | quick=1, standard=2, thorough=3 |
| 用例角色 | search=1, validation=2 |
| 备选类别 | inlining=1, specialization=2, unrolling=3, loop-SIMD=4, SLP=5, short-slice/versioning=6, layout=7 |
| 展开处置 | legal=1, illegal=2, duplicate=3, growth-rejected=4 |
| 候选结果 | baseline=1, compiled-unmeasured=2, size-rejected=3, timed-out=4, search-nonwinner=5, validation-threshold=6, validation-nonwinner=7, selected=8 |
| 顺序阶段 | candidate-smoke=1, search-warmup=2, search-measured=3, validation-one-warmup=4, validation-one-measured=5, validation-two-warmup=6, validation-two-measured=7；仅 3、5、7 出现在存储流 |
| 选择原因 | tuned=1, no-candidate=2, validation-threshold=3, validation-disagreement=4 |
| 输出角色 | primary=1, header=2, import-library=3 |
| 缓存来源类型 | freshly-built=1, verified-local-hit=2 |
| 诊断码 | none=0, legality-rejected=1, growth-rejected=2, artifact-size-rejected=3, candidate-timeout=4 |

集合顺序为：用例按 id；Site、Unit、Variant 按稳定 id；展开按序号；候选先
基线再按计划摘要；计划选择按应用阶段再按单元 id；流按阶段、轮、用例 id、计划摘要；流内
行按序号；输出按角色。Text 比较和排序以编码后的 UTF-8 字节为准。

仓库携带五个规范 Schema Fixture：

- `tests/fixtures/tune/decision-schema1-framing.hex`；
- `tests/fixtures/tune/decision-schema1-baseline.cktune`；
- `tests/fixtures/tune/decision-schema1-tuned.cktune`；
- `tests/fixtures/tune/decision-schema1-inspection.json`；
- `tests/fixtures/tune/decision-schema1-inspection.txt`。

Framing Vector 覆盖全部标量/容器类型和两种可选状态。基线 Vector 有一个搜索
与一个验证用例以及 `no-candidate`；调优 Vector 有一个单元、一个合法 Variant、
完整三阶段样本和有效证书。Parser 实现验收前，字节及 SHA-256 必须冻结在
Schema 测试中；编码、解码、检查、重编码、截断、突变和跨端序测试共用它们。

### 12.3 记录的身份

文件记录：

- CK 编译器版本与源码身份；
- Rust 工具链、LLVM 和 LLVM Bridge 身份；
- 语言、原生 ABI、运行时 ABI、KIR、证明、成本模型、目标和缓存 Schema；
- 源码、语义、调优前 KIR、契约和模式摘要；
- 输出类型、精确宿主目标三元组、规范化 CPU、特性集合和目标 Profile；
- 可选 .ckprof 身份与摘要，或者明确记录不存在；
- 规范清单、驱动程序、允许环境变量和声明输入摘要；
- 预算预设、候选空间摘要和测量策略摘要；
- 测量环境的操作系统、内核、硬件、计时器和拓扑证据；
- 每个候选计划、目标图摘要、产物体积、拒绝原因、正确性摘要、原始存储
  样本和稳定性结果；
- 两轮验证决策；
- 获选计划或者规范的基线选择原因；
- 完整的带角色暂存输出集字节摘要与物理体积；
- 跨冷搜索比较所用、与测量无关的获选代码身份；
- 不可变的编译与测量缓存来源事实。

原始工作负载文件、任意驱动 stdout、秘密和绝对路径不得存入规范身份。调优
命令运行时，人类诊断可以显示明确标注为非规范的本地路径。

测量环境 Tuple 是封闭的：操作系统系列与 Build、内核版本、架构、CPU
Vendor/Family/Model/Stepping、宿主可提供时的 Microcode、规范化 CPU 特性、
物理核/逻辑核和 NUMA 拓扑，以及单调计时器种类与报告分辨率。不可获得的
字段使用一个显式 unavailable 值，不能省略。禁止记录主机名、用户名、硬件
序列号和操作系统机器标识。

### 12.4 重放身份

重放不要求原清单、驱动程序或工作负载输入存在，但必须精确匹配：

- 编译器和全部相关 Schema；
- 源码、语义、契约和调优前 KIR；
- 目标三元组、原生 CPU、特性和目标 Profile；
- 可选 profile 身份或者明确不存在；
- 编译模式和输出类型；
- 决策前沿、前置条件和规范获选计划。

规范 .cktune 决策摘要进入生产原生缓存键。记录的输出集摘要用于验证原始发布
配对以及任何完整决策 Cache Hit。后续 tune-use 可以使用不同目标 Basename；
它必须精确复现记录的目标图和链接配方，而由目标路径衍生的打包字节由该次构建
重新审计与记录，不与原始路径相关容器摘要比较。已有决策中的缓存来源事实不可
变，重编码或复用不得重写它。编译器、Schema、源码、CPU、
特性、profile、模式或者计划发生变化时，必须重新调优。选择基线的决策包含
空覆盖计划；tune-use 正常验证该决策，然后复现精确的普通基线。

## 13. 编译与重放流水线

调优流水线为：

1. 解析并计算编译器、源码、目标、模式、清单、驱动程序、输入和可选 profile
   的摘要。
2. 构建并完整验证精确的普通基线。
3. 枚举稳定决策点、调优单元、备选项和候选前沿。
4. 在所选预设下运行确定性 Beam Search。
5. 将合法备选编译为不可发布的试验产物。
6. 完整测量前运行正确性冒烟检查。
7. 测量搜索最终候选。
8. 在不同验证用例上完整验证领先入围者两次。
9. 为获胜精确计划签发测量收益证书，或者记录基线原因。
10. 从调优前编译器状态独立重放获选计划。
11. 重新构建并验证获选目标图。
12. 将规范目标图与链接配方摘要同被测候选比较。
13. 按第 6 节带 Journal、产物最后发布的协议发布最终产物与决策。

后续 tune-use 构建使用决策文件重复第 1、9 至 12 步。每个决策点、候选前沿、
前置条件、前状态、后状态、目标图和链接配方摘要都必须匹配。

被测候选与最终代码目标图必须相同。仅当打包路径、时间戳和平台签名容器不
可能影响加载代码时，才可将其排除在规范比较外；每个平台都必须显式记录并
测试这种排除。

## 14. 缓存与中断会话

调优数据位于现有 CK 缓存根目录下的 tune-v1。默认调优缓存硬上限为 4 GiB。

缓存分离：

- 编译身份：代码身份加精确计划；
- 测量身份：产物、驱动程序、工作负载、环境和策略。

测量键还包含一个随机生成、以私有权限保存的本地缓存安装 Salt。原始 Salt 不写入
.cktune，但记录其域分离摘要以便重新推导缓存来源。因此，不能仅因为另一台机器报告相同 CPU 型号和操作系统 Tuple，
就复用其原始测量；只要目标在其他方面精确兼容，移动决策文件后显式
tune-use 仍然允许。

条目使用私有权限、校验和、规范校验路径、原子发布和确定性 LRU 淘汰。符号
链接和路径穿越攻击必须拒绝。

精确且完整的决策可被热缓存 tune build 复用。no-tune-cache 强制重新搜索
与测量。中断会话只能复用已经完整验证的已编译候选。未完成的测量阶段必须
丢弃并从第零行重新开始，绝不能拼接不同会话的样本。

完整的基线决策可以记录候选超时，但该完整决策不能作为完整决策 Cache Hit
复用。崩溃、协议错误、摘要不匹配、语义不匹配和部分决策也不得缓存为成功
结果。ckc cache clean 使用已有安全根目录保护，同时删除普通缓存和调优缓存。

发布的产物和决策文件不依赖缓存继续存在。

## 15. 安全、隐私与失败行为

- 调优不执行遥测、网络上传、Profile 服务或远程执行。
- 驱动程序是用户显式授权的可执行文件，不使用 shell 插值。
- 输入和输出在分配内存前执行边界检查。
- 临时目录和协作式进程组/Job Object 由会话在第 8.3 节明确边界内拥有并清理。
- 最终输出集使用第 6 节带 Journal、摘要校验且主输出最后发布的协议。
- 公共解析器接受模糊测试和变异测试。
- 试验类型状态从构造上保证未验证产物不可发布。

以下情况中止整个命令且不产生新输出：

- 配置或身份无效；
- 基线编译或验证失败；
- 驱动程序崩溃、信号终止、协议格式错误、输出超限或摘要错误；
- 必要测量不稳定；
- 无法完成固定验证；
- 合法计划的编译、验证或者重放不匹配；
- 目标图或者链接配方不匹配；
- 任意内部不变量或者算术溢出错误。

只有两种结果是普通成功：合格的调优产物；或者测量充分、但没有候选通过
阈值时的基线产物。

## 16. 诊断与检查

文本和 JSON 检查显示：

- 全部编译器、Schema、源码、profile、目标、CPU、特性和模式身份；
- 清单与测量策略身份；
- 调优单元、备选项、候选、拒绝和计划选择；
- 校准、原始存储样本、中位数、稳定性、正确性摘要和加权得分；
- 两轮验证决策和阈值；
- 获选计划或者基线原因；
- 与测量无关的选择身份；
- 编译与测量缓存复用；
- 最终重放和目标图验证。

JSON 与默认文本使用
[`inspection-schema-1.md`](../inspection-schema-1.md) 规定的精确字节和完整树
遍历。确定性输出不包含绝对路径、时间戳、临时标识、Hash Map 顺序或本地化文字。

tune-use 与 explain-optimization 组合时，每个获选项映射回其稳定决策点、
静态预测、测量证据、保护和重放结果。

## 17. 版本与 ABI 契约

CK 0.14 改变优化和缓存行为，但不改变语言或运行时 ABI。

要求的 Schema 状态为：

| 契约 | CK 0.14 值 |
| --- | ---: |
| 语言契约 | 相对 v0.13 不变 |
| 原生 ABI | 1 |
| 运行时 ABI | 2 |
| KIR 格式 | 3 |
| LLVM Bridge ABI | 4 |
| CK Profile Schema | 1 |
| Multiversion Schema | 1 |
| 原生缓存条目魔数 | CKCOBJ04 |
| 原生缓存键与 Manifest | 5 |
| 调优输入 Manifest | 1 |
| .cktune 格式、契约、测量、检查与计划 | 1 |

不得新增运行时符号或共享库依赖。调优 Schema 常量集中定义，并由不匹配和
变异测试覆盖。

CK 0.14 将 CKCOBJ03/schema 4 条目视为干净的 Cache Miss，绝不原地升级，也
不按 Schema 5 解释。

## 18. 测试与 CI 要求

现有十个必需 Job 继续保持必需：

- quality；
- native integration；
- 六个原生宿主 Job；
- 两个稳定性能 Job。

全部 Job 针对精确候选 SHA 运行，不得使用 continue-on-error，也不得静默
跳过必需能力。

该矩阵同时是 native runtime 与 backend 的 portability gate。以下要求会阻断发布，
不得作为 host-specific test exception 处理：

- Profile runtime 使用显式 internal atomic abstraction；Windows 使用受支持的 Interlocked
  operation，不假定 MSVC 已启用 C11 atomics；全部宿主保持相同 acquire/release publication
  model；
- Linux x86-64/AArch64 与 Darwin x86-64/AArch64 必须通过各自 platform adapter 持久发布并
  重新打开 profile shard；directory、open、identity、write、rename 与 sync failure 在映射到
  稳定 public status 前保留不同 internal cause；
- Artifact assertion 从 `NativeArtifactPaths` 推导宿主文件名，不得在 Linux 或 Windows 上
  hard-code Darwin 扩展名；
- LLVM call lowering 绝不为 `void` call 分配 value name，并以曾触发该断言的 PGO/tuning
  fixture 建立回归测试；
- 全部六个 native-host Job 都编译 Profile runtime、运行精确 publication test，并构建
  executable 与 dynamic artifact，之后才可认为该平台受支持。

六个原生宿主验证：

- Manifest 与决策解析；
- 可执行文件与动态库驱动协议；
- 确定性搜索与重放；
- 协作式进程组/Job Object 超时与清理，并验证敌意 POSIX 逃逸不属于驱动契约；
- 缓存权限、失效、损坏、路径穿越与淘汰；
- 每个阶段边界的 Journal 恢复、回滚/前滚、完整输出集摘要验证和主输出最后发布；
- 普通非调优行为；
- 最终产物保持现有自包含系统运行时策略。

性能声明只能基于稳定 Linux 增强型 x86-64 与 AArch64 Worker。性能输出升级至
Schema 9。CI 还包括：

- 解析器、规划器和进程控制的消毒器与 ASan 覆盖；
- 决策文件与 Manifest 模糊测试；
- Schema、身份、摘要、阈值和类型状态检查的变异测试；
- 大小端、截断、重复、尾随和超限输入的固定 Fixture；
- 被终止会话恢复与样本不可拼接测试；
- 确定性冷缓存与热缓存测试；
- 证明 tune-use 失败关闭的负向测试；
- 证明普通构建不读取调优状态的测试。

全部本地和远程门槛通过前，不得创建 CK 0.14 Tag 或 Release。

## 19. 性能验收

CK 0.14 保留所有已经通过验收的 v0.12 和 v0.13 正确性、代码质量、性能、
编译时间和产物门槛。即使调优基准改善，对这些门槛的回归也会阻止发布。

### 19.1 冻结 Schema 9 证据契约

英中设计共用的规范附件
[`performance-schema-9.md`](../performance-schema-9.md) 是唯一逐字段 JSON
权威；它固定所有嵌套键、类型、数量、统计、身份和失败关闭检查。本节固定相关
产品策略与仓库资产。

Schema 9 扩展但绝不替代两个不同的 Schema 8 门槛。由
`benches/baselines/v0_13_replay.toml` 指定的历史已验收报告，必须在精确 v0.13
Detached Checkout 中由其保留 Checker 验证；另一个新鲜累计兼容报告，以候选
版本 0.14.0 和当前候选 SHA 在显式兼容模式下重跑整个 Schema 8 套件，全部旧阈值
保持不变。禁止改写历史报告或在 v0.14 HEAD 下直接检查它。五个可调优用例恰为
`benches/cases/pgo-cases.tsv` 的五行：`branch-layout`、
`call-constant-length`、`trip-unroll-simd`、`memory-bound`、`compute-bound`。
没有可选行或结果产生后的排除。

现有 `training.tsv` 是搜索输入，`held-out.tsv` 是验证输入；调优器通过七个固定
`benches/tune/workloads/*.cktune.toml` Manifest 接收二者。调优器永不接收
封存发布文件 `benches/fixtures/tune/release-held-out.tsv`，其精确数据行为：

    ckc-tune-inputs\t1\trelease-held-out
    branch-layout\trelease-branch-prime\t16381\t79\t3
    call-constant-length\trelease-fixed-4000\t4000\t83\t13
    trip-unroll-simd\trelease-map-4093\t4093\t89\t0
    memory-bound\trelease-zip-4096\t4096\t97\t0
    compute-bound\trelease-f64-4091\t4091\t101\t1.0009765625

`benches/cases/tune-cases.tsv` 在 Schema Header 后严格包含以下七个逻辑行；
每行固定源码、Manifest Basename 以及搜索/验证/发布记录来源：

| 调优用例 | 源码 | 搜索记录 | 验证记录 | 发布记录 |
| --- | --- | --- | --- | --- |
| branch-layout | `benches/fixtures/pgo/branch_layout.ck` | train-branch-biased | held-branch-prime | release-branch-prime |
| call-constant-length | `benches/fixtures/pgo/call_constant_length.ck` | train-fixed-4000 | held-fixed-4000 | release-fixed-4000 |
| trip-unroll-simd | `benches/oracles/fixtures/map_u32.ck` | train-map-4000 | held-map-3967 | release-map-4093 |
| memory-bound | `benches/oracles/fixtures/zip_u32.ck` | train-zip-4000 | held-zip-4000 | release-zip-4096 |
| compute-bound | `benches/fixtures/pgo/compute_bound.ck` | train-f64-4000 | held-f64-3989 | release-f64-4091 |
| contract-noalias | `benches/oracles/fixtures/contract_noalias.ck` | train-zip-4000 | held-zip-4000 | release-zip-4096 |
| contract-fixed-length | `benches/oracles/fixtures/contract_fixed_length.ck` | train-fixed-4000 | held-fixed-4000 | release-fixed-4000 |

每行的 `<tune-case>.cktune.toml` 严格为 Schema 1：runner path 是
`../../../target/release/ckc-tune-runner`，input root 为 `../..`，args 为
`["--ck-tune"]`，inputs 为
`["fixtures/pgo/training.tsv","fixtures/pgo/held-out.tsv"]`，
`inherit_env` 为空，`timeout_ms=30000`。它恰含 `<tune-case>.search` 与
`<tune-case>.validation`，Role 分别为 search/validation，Weight 为 1，Seed
来自命名输入记录。Expected Digest 不是实现可选择常量，而是：

    SHA-256("CK-TUNE-RESULT\0" || U32_BE(native_abi_schema) ||
            U32_BE(len(case_id_utf8)) || case_id_utf8 ||
            U64_BE(result_byte_count) || canonical_result_bytes)

结果同时写入 `tune-cases.tsv` 与 Manifest。收集证据前，经审计的 CK、C、Rust
实现必须独立产生这些精确字节。发布摘要使用 `<tune-case>.release` 作为 case
id；发布记录与摘要绝不出现在调优 Manifest 中。

固定 Recipe 包含 `benches/cases/tune-cases.tsv`、七个工作负载 Manifest、
`benches/tune/runner.rs`、`benches/oracles/tune/manifest.toml`、
`benches/oracles/tune/c/tune_oracle.c`、
`benches/oracles/tune/rust/tune_oracle.rs`、由五个用例以及
`contract_noalias.ck`、`contract_fixed_length.ck` 组成的七份 CK 源码、四个
输入分区、`benches/tune_perf.rs`、`scripts/measure-v014-performance.py`、
`scripts/check-native-performance.py`、`scripts/audit-performance-oracles.py`、
`scripts/package-v014-performance-archive.py`、`LICENSE` 和
`THIRD_PARTY_NOTICES.md`，以及 `benches/baselines/v0_13_replay.toml` 和规范
`specs/0.14/performance-schema-9.md`。
Oracle Manifest 固定 C11、Rust 2024、严格浮点行为、安全前置条件与 UB/别名
审计。任意 Recipe 字节变化都会使证据失效。

报告路径为 `target/ckc-perf/v0.14-results.json`；证据位于其旁边名为
`v014-measurement-<unix-seconds>-<pid>` 的真实、非符号链接目录。顶层键集合
严格为：

    schemaVersion, candidateVersion, candidateSha, v013ReplayCommit,
    evidenceDirectory, toolchain, hardware, recipe, candidateBinary,
    v013ReplayBundle, cumulativeSchemaEight, workload, tuningDecisions,
    tuningArtifacts, sampling, cases, validationCases, domainCases, tuneUseCompileTime,
    ordinaryCompileRegression, artifactSize, archiveSize, resourceUse,
    determinism, correctness

`schemaVersion` 为 9，`candidateVersion` 为 `0.14.0`。候选 SHA、编译器字节、
精确 v0.13 历史 Replay 证据闭包、新鲜 v0.14 Schema-8 兼容证据闭包、固定
LLVM/Clang 22.1.8、Rust 1.90.0、保留的 `/usr/bin/ld` 系统链接器身份、驱动字节、
Manifest、源码/输入字节、硬件、
操作系统、CPU 特性、Recipe、产物、决策和每个保留证据文件都具有体积与
SHA-256 身份。每个证据根条目
必须是证据目录下的普通文件，不得有符号链接、路径穿越、缺失、重复或未知项；
仓库根身份只能在干净的候选 Checkout 中解析。
稳定 Linux x86-64 Worker 必须具备 x86-64-v4；稳定 Linux AArch64 Worker 必须具备 SVE2。
缺少 Tier 属于门槛失败，绝不是 Workflow 可选择项。

主用例计时使用 `rotating-six-channel-v1`，通道顺序严格为：`tuned`、
`v014Ordinary`、`v013Ordinary`、`v013Pgo`、`cSimd`、`rustSimd`。验证计时
使用 `rotating-three-channel-v1`，通道为 `tuned`、`v013Ordinary` 和
`v013Pgo`。领域计时
使用 `rotating-three-channel-v1`，顺序为 `tuned`、`genericC`、
`genericRust`。三者都执行并保留三行不计分预热的回执、保留二十行测量；每个样本将每通道
调用七个等量批次并存最小值，结果使用上中位数。至少 16/20 样本必须位于中位
数 80%..120% 闭区间。轮转由候选、用例、分区和行摘要导出。动态加载、符号
解析、准备、调优搜索与驱动 I/O 不计入稳态计时。禁止选择性重跑。`.cktune`
内部三调用决策证据与外部七调用发布样本彼此独立。
每个外部回执只在保留的原生 runner 迭代循环内部计时。Collector 以空环境直接
启动 runner 并解析严格的 `CKPERF/1` 回执；进程启动、动态加载、输入分配、结果
哈希、输出以及 Python/FFI 循环开销均不计入所报告时间。
每个 C/Rust Oracle 构建也以严格空环境启动。C Oracle 通过 Clang `--ld-path`
显式指定独立解析的 `/usr/bin/ld`；Rust Oracle 通过显式 `-C linker` 与
`-C link-arg` 指定已解析的固定 Clang Driver 和同一链接器。其现场字节必须等于
保留的工具链身份，因此 Oracle 构建不会通过 PATH 解析链接器。

每个用例/分区先记录固定倍增校准和一次确认；获选的每调用迭代数在全部通道、
预热回执和测量回执中相同。每个回执记录请求/完成迭代数、时间和正确性。验证
回执必须等于 Manifest 期望摘要；发布/领域回执必须等于独立再生成的冻结结果。

每个主用例记录全部六个原始七调用流及顺序、逐行最小值、中位数、各通道正确性
摘要、源码/输入身份、
获选或基线决策、完整 `.cktune` 身份、全部产物身份、固定为 true 的 Eligibility
位与发布留出结果。全部七个验证用例都在 Manifest 验证输入上记录 tuned、
v0.13 ordinary 和 v0.13 PGO 原始七调用流及各通道正确性摘要；检查器选择更快的 v0.13 中位数并应用不变的
102/100 上限。两个领域用例记录三通道的同类事实以及精确调优决策、输出
集与三条构建命令。全部七个 `.cktune` 和完整带角色发布输出集复制进证据；其
Schema、身份、证书/基线原因、计划、目标图、
链接配方、测量与磁盘摘要均被独立检查。

tune-use 编译时间相对 v0.14 普通编译测量：三对预热、十五对测量，每行交替
首通道；v0.14 普通编译与精确 v0.13 普通编译使用同一协议。两者使用上中位数，
以 `TimedCommand` 回执逐项绑定并保留原始时间与命令身份，使用已终止子进程的
user+system CPU time，排除托管 runner 被调度移出的时间但不移除任何编译器工作，
并排除调优搜索。产物体积使用与计时配对的精确主输出。
资源证据保留 standard 会话墙钟、编译器/调优器峰值 RSS、展开/编译/最终候选
计数以及缓存字节。确定性证据以与测量无关的选择身份、计划、目标图、链接配方
和发布内容身份比较两个独立冷缓存会话，再以决策与输出字节以及零编译/测量计数
证明一次精确热缓存复用。真实冷会话的原始计时必须保留，并允许不同。
每个会话都保留精确 Tune Build 命令、加锁的完整 Cache 前后清单、规范事件日志、
原始计数、墙钟、峰值 RSS、决策和输出。两个冷 Namespace 不同且初始为空；热
运行在无中间访问的条件下从 Cold One 的精确运行后 Cache 清单开始。
两个稳定 Linux 性能宿主都用同一个 direct-child `wait4` Supervisor 测量 tuned 与
ordinary 编译器进程；保留回执绑定精确命令、`CLOCK_MONOTONIC_RAW` 区间、零等待
状态和由 KiB 转换为字节的内核 `ru_maxrss` 高水位。稀疏轮询不能作为峰值内存来源。
每个计时通道都携带封闭构建命令，并通过显式外键链连接其输出字节与调优
决策/输出集或审计基线。全部 CK 性能构建显式使用
`--overflow unchecked --bounds unchecked`，每个 Oracle 通道固定相同的已定义
输入语义；比较不依赖 CLI 默认值，也不混合安全模式。规范调优决策/输出集就是
第一次冷确定性运行；主/验证计时、产物体积、资源与热复用记录都引用同一保留
身份，而不是互不绑定的副本。
每个文件身份明确选择候选 SHA 仓库根或保留证据根。

Archive 体积比较 Replay Manifest 的精确 v0.13 Archive 与确定性三成员 v0.14
Archive；后者包含上述同一候选编译器、仓库 License 与 Notices，并保留 Recipe
固定的 Producer、封闭调用、完整成员清单、元数据、压缩字节和静态依赖审计。

`scripts/measure-v014-performance.py` 只收集原始证据；
`scripts/check-native-performance.py` 是唯一验收权威，对任何缺失/未知键、身份
不匹配、非有限或非正测量、错误数量/顺序、不稳定流、不合格硬件、决策不匹配、
阈值失败、选择性重跑或未保留证据都失败关闭。
`scripts/audit-performance-oracles.py` 独立复核源码/Oracle/输入覆盖与语义。两个
必需稳定性能 Job 在 Linux 增强型 x86-64 与 AArch64 宿主上、同一候选 SHA 执行完整
契约。

### 19.2 冻结阈值

上述语料在测量前划分为搜索、验证和封存留出用例。可调优用例和排除项在结果
产生前声明，禁止测量后排除。

对可调优用例，将获选调优结果与相同语义下更快的 v0.13 普通或 PGO 原生
基线比较：

- 留出集几何平均至少快 5%；
- 每个获选用例至少快 2%；
- 任一验证或留出用例的减速不超过 2%。

测量前声明为可调优的每个语料成员都参与验收，包括调优器选择基线的成员。
因此，不得在结果产生后排除选择基线的成员来改善发布结果。

与相同语义和硬件下、更快且经过审计的手写 C 或 Rust 加显式 SIMD 参考比较：

- 几何平均性能至少达到 98%；
- 每个用例至少达到 92%。

在冻结的两用例领域约束套件 `contract_noalias.ck` 和
`contract_fixed_length.ck` 上，调优 CK 相对语义通用且更快的 C 或 Rust O3
结果，几何平均快 8% 以上；两个用例都必须参与。

资源与确定性门槛为：

- 调优产物字节数不超过匹配基线的 110%；
- 排除调优搜索后，tune-use 编译相对同一非 tune-use 构建的开销，几何平均
  不超过 10%，任一用例不超过 20%；
- 普通非调优编译回归，几何平均不超过 3%，任一用例不超过 8%；
- 编译器归档体积不超过已验收 v0.13 归档的 110%；
- standard 会话在 30 分钟及其声明候选限制内完成；
- 调优器峰值编译器 RSS 不超过对应普通编译的两倍；
- 调优缓存绝不超过 4 GiB 硬上限；
- 两次冷运行具有相同的测量无关选择身份、计划、目标图、链接配方和发布输出
  内容；真实校准与计时证据可使原始决策摘要不同；
- 精确热缓存复用编译和测量零个候选，并逐字复现第一次冷运行的决策和全部
  带角色输出；
- 最终产物不包含调优驱动程序、调优符号、运行时分派或新的运行时依赖。

发布证据记录硬件、操作系统、编译器身份、原始样本、排除项和精确产物摘要。

### 19.3 Predicated-update 优化兑现门槛

Schema 9 及其冻结语料保持不变。此外，两个稳定 Linux 性能宿主都运行一个独立的
fail-closed gate，证明新的 predicated-update 能力确实被选择且具有收益，而不只是代码中
存在。源码为 strict-`f64` Floyd-Warshall kernel，其内层循环采用规范 same-place
conditional update。两个 channel 的源语言、ABI、safety mode、target、CPU policy、LLVM
pipeline 与 input generator 完全相同。
该独立 report 的逐字段定义、recipe、runner、attestation、sampling、evidence closure、checker
与 CI invocation 的唯一权威是
[`predicated-update-performance-1.md`](../predicated-update-performance-1.md)。

语料在测量前固定：PGO generation 与 tuning search 使用确定性的 `N=128`、seed-113
matrix；tuning validation 使用互不重叠的确定性 `N=256`、seed-127 matrix；release timing
使用封存的确定性 `N=1024`、seed-131 matrix。Generator 在对角线上生成零，为存在的边生成
有限非负权重，为缺失边生成正无穷，因此契约不含 negative cycle 或 NaN input。Seed、
source bytes、generator bytes、profile、manifest、decision、artifact、
compiler、LLVM、hardware capability、command、correctness digest 与全部 raw timing receipt
都必须保留。Training/validation input 不得作为 release timing evidence。

Release 恰好包含两个 channel：

- `pgoOnly`：`ckc build`、O3、native CPU、显式 PGO-use、
  `--overflow unchecked --bounds unchecked`，且不使用 tune-use；
- `pgoTuned`：相同 build identity 与 PGO profile，再应用
  `ckc tune build --pgo-use` 产生的 decision。

Decision 必须选择恰好含一个 PlanChoice 的非 baseline plan。该 choice 必须解析为恰好含一个
SiteAlternative 的 Loop SIMD UnitVariant，且该 alternative 正是已验证的 predicated-update；
不得同时包含 layout、short-slice、第二个 Loop SIMD 或任何其他 choice。source-aware checker
还必须证明记录的 minimum 不大于 128，并证明固定 N/slice 事实使全部 runtime legality guard
为真，且 training、validation、release 均至少执行一个 vector chunk。Baseline decision、复合
plan、动态不可达的改写、stale profile/decision，或不能精确 replay 的 decision 都使门槛失败。
任何 timing 被采纳前，correctness 必须与独立 scalar oracle 一致。
另设 checked mode 正向与负向 optimizer test：只有 lane bound、overflow 与 first-failure
ordering 全部得到证明时才接受同一改写，否则保持 scalar。

Timing 先运行三个不计分 warm-up row，再运行二十个 measured row，确定性轮换首个 channel，
保留全部 monotonic-clock raw receipt，使用 upper median，并分别对两个 stream 应用 schema-9
的 16-of-20、闭区间 80%..120% stability rule。每个宿主上的
`pgoTuned/pgoOnly` 必须不超过 `95/100`，任一 validation case 不得超过 `102/100`。Failure、
instability、缺少 evidence 或事后排除都会阻断发布。该门槛叠加于全部 schema-8/schema-9、
resource、size、determinism 与十 Job CI 要求之上，彼此不得抵销。

## 20. 发布门槛

只有满足以下全部条件，CK 0.14 才可发布：

1. 集成最终通过验收的 v0.13 基线，且所有继承门槛继续为绿；
2. 本规范中的每项规范行为都有正向和负向测试；
3. 从干净 Checkout 通过本地总验收；
4. 十个精确 SHA 远程 Job 全部通过；
5. 冻结 Schema-9 语料与独立 predicated-update 语料达到第 19 节全部阈值；
6. 文档、CLI Help、示例、Schema 和检查输出一致；
7. 生成的可执行文件和动态库保持承诺的零额外依赖部署模型；
8. 仓库干净，全部发布证据由精确 SHA 的 CI Run 与 Release Archive 保留；生成
   的证据不提交进其自身所记录的源码 Commit。

只通过本地功能测试、只通过搜索工作负载或者只通过一个性能宿主，都不足以
发布。

## 21. 风险与控制

| 风险 | 必需控制 |
| --- | --- |
| 工作负载过拟合 | 不同的搜索与验证用例、两轮验证以及封存发布留出用例 |
| 测量噪声 | 外部单调计时、校准批次、轮转、固定样本、稳定性门槛和禁止挑选式重跑 |
| 测量选择不安全 | 合法性独立于收益、试验类型状态、精确重放与最终独立验证 |
| 组合爆炸 | 稳定调优单元、有限备选、Beam Search、封闭预设与硬候选上限 |
| 复用过期决策 | 完整编译器、源码、目标、profile、模式和前沿身份以及失败关闭重放 |
| 缓存投毒 | 私有根目录、规范路径、校验和、有界解析、原子条目与身份分离 |
| 驱动程序危害 | 显式用户授权、无 shell、清空环境、有界 I/O 与真实说明不提供沙箱 |
| 隐藏的普通构建成本 | 仅显式集成与普通编译回归门槛 |
| 复现被测产物 | 规范计划、目标图与链接配方摘要以及确定性重放 |

## 22. 延后演进

后续设计可以考虑多 CPU 或集群调优、可移植基线调优、静态库、目标文件、
交叉编译、可扩展 KIR、间接调用提升、自适应 ORC JIT、源码 SIMD、宽松浮点、
GPU 目标、远程服务或者遥测。

它们需要单独的语言、安全、身份、可复现性和验收决策。CK 0.14 调优 Schema
不对这些能力作兼容性承诺。

## 23. 设计完成标准

本设计只有在英文与中文文档同时满足以下条件时才算完整：

- 描述相同的规范行为和常量；
- 不包含未解决选择、占位符或“由实现决定”的逃生条款；
- 保留全部 v0.13 语言、安全、ABI 和发布契约；
- 从显式工作负载、合法搜索、测量、验证、证书、确定性重放、缓存、检查到
  发布形成完整闭环；
- 可以直接拆解实现计划，而无需再发明产品策略。

在本设计完成审阅和批准，并解决临时 v0.13 基线条件前，不得从该分支开始
实现。
