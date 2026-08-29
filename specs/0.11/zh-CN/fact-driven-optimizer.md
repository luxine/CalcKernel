# CK 0.11 事实驱动优化器规范

[English](../fact-driven-optimizer.md)

本文是 CK 0.11 优化器基础的预发布规范，不改变已发布的 0.10 语言、CLI、MIR
或 ABI 契约。0.11 实现并发布后，其中的持久要求应并入当前双语文档树，同时删
除这份预发布规范。

## 目标

CK 不再把 LLVM 视为唯一的优化知识来源。编译器将推导并保留普通 C/Rust 编译
通常无法可靠恢复的事实：数值范围、内存区域、别名关系、访问效果、对齐、slice
边界、循环迭代范围和调用效果。CK 在自己的目标无关优化器中证明变换，只向后端
工具链传递经过验证的事实。

长期性能目标在同算法、同安全语义、同可观察行为和同硬件下衡量。合格 Kernel
应接近固定且经过审计的手写 C/Rust+SIMD 实现；当 CK 契约暴露通用编译器无法获
得的领域约束时，应超过固定的通用 Clang/Rust O3 参考。不得通过削弱严格浮点、
checked 首错顺序、print 顺序、ABI 或声明的契约定义域来达到目标。

## 选定架构

编译器采用证据优先的统一优化 IR：

```text
AST 与已检查的源码契约
          |
          v
       语义 MIR
          |
          v
   KIR 构造器与验证器
          |
          v
KIR：标量 SSA + 区域 Memory SSA + 事实 + 效果 + 证明
          |
          v
       KIR 优化器
      /      |       \
     C   WebAssembly  Native LLVM
```

语义 MIR 继续拥有源码求值顺序、checked 首错顺序、runtime print 顺序、前端无
关语义和稳定的 `emit-mir` 文本。KIR 是目标无关分析和优化的唯一内部表示。0.11
发布前，C、WebAssembly 和 Native LLVM 后端都必须消费经过验证的 KIR。迁移期
可以使用仅供开发的 shadow 管线对比 KIR 与 0.10 路径，但发布版不得永久保留两
套优化管线。

KIR 文本确定且可检查，但它是内部编译器格式，不承诺跨版本兼容。

## 可信源码契约

### 语法与 unsafe 边界

可信事实只能附加在 `unsafe fn` 入口：

```ck
export unsafe fn saxpy(
    x: slice<f64>,
    y: slice<f64>,
    n: u32
) -> void
contract {
    requires n <= x.len && n <= y.len;
    requires noalias(x, y);
    requires aligned(x.data, 32);
    effects read(x), write(y);
}
{
    // function body
}
```

每次调用 unsafe 函数都必须位于显式 unsafe 语句块内，包括另一个 unsafe 函数
中的调用：

```ck
unsafe {
    saxpy(x, y, n);
}
```

unsafe 函数至少要有一个 `requires` 子句。安全函数不能带 contract 或 effects
子句。unsafe 块不会抑制其他类型、控制流、bounds mode 或 ABI 诊断。

控制流进入函数时契约必须成立。任一 `requires` 为假会使该次执行立即成为 UB，
与优化级别、后端或某个 pass 是否恰好使用该事实无关。正常 O0–O3 编译不会插入
契约检查。

这个边界管理新增的可信优化事实，并不追溯性地把 CK 变成内存安全语言：裸指针
和 `slice(data, len)` 的既有内存有效性仍由调用者负责。反过来，普通 CK 执行也
不会仅因优化器猜测而得到新的 UB；变换使用的每个非契约事实都必须被证明。

`unsafe` 不改变函数的 C ABI。导出的 unsafe 函数对应的生成头文件必须包含规范
化契约注释，外部调用者承担相同的入口义务。加强已导出函数的前置条件是破坏性
源码契约变更；放宽前置条件兼容。

### 封闭契约语言

契约表达式是无副作用的编译器事实，不是可执行 CK 代码。它们可以包含：

- 整数参数、整数常量、`slice.len`，以及指针谓词需要的 `slice.data`；
- 使用加法、减法和整数常量乘法的仿射整数表达式；
- `==`、`!=`、`<`、`<=`、`>`、`>=` 和逻辑合取；
- `multiple_of(value, positive_constant)`；
- `noalias(slice_a, slice_b)`；
- `aligned(pointer, power_of_two)`；
- 一个可选效果上限：`effects none`，或由 `read(slice)`、`write(slice)`、
  `readwrite(slice)` 组成的逗号分隔集合。

函数调用、析取、否定、内存读取、存储、可变状态和目标相关缓存、向量宽度或预
取提示都不是 0.11 的契约语法。契约整数表达式在无界数学整数上解释，因此求值
本身不会溢出。在提升为数学整数前仍会执行普通 CK 类型规则；契约语言不会产生
隐式 signed/unsigned 转换。

`noalias(a, b)` 表示两个 slice 描述符所表示的完整有效字节区间在本次调用的动
态期间不重叠。这些区间是数学分配区间，不会在目标地址宽度处环绕。零长度 slice
表示空区间。`aligned(p, n)` 要求 `n` 是不超过 `2^31` 的正 `u32` 2 的幂，且指
针地址能被 `n` 整除；该谓词认为空指针对齐，而其可解引用性继续遵循既有 slice
有效性规则。0.11 中 effects 的目标必须是具名 slice 参数。

effects 子句是允许效果的上限，不是未经检查的承诺。编译器会根据函数体和传递
callee 摘要检查它。省略该子句表示请求自动推导。声明不完整产生 `CK2016`，而
不是 UB。

0.11 不包含局部 `assume`、循环契约或任意契约表达式。循环事实必须从入口契约、
SSA 值、分支条件和归纳分析中推导。

## KIR 模型

KIR 包含有类型函数、基本块、标量 SSA 定义、phi 节点、显式控制流、区域
Memory SSA，以及可能失败或产生 runtime 效果的显式操作。在携带证明的变换删
除 bounds/overflow guard 前，这些 guard 始终显式存在。潜在 checked 失败和
runtime print 都是有序效果；没有不可观察性证明时，变换不能把其他潜在失败或
可观察操作跨过它们移动。

每个 pointer 或 slice 来源都有稳定的 `MemoryRegion`。sub-slice 保留父区域和
符号字节区间。已证明的 `noalias` 关系对区域分区。load 消费某个分区版本，store
和有效果调用产生新版本，控制流汇合产生 memory phi，未知别名会把相关区域合并
到保守分区。无法证明分离只会损失优化机会，不会损失正确性。

事实在单次编译中有稳定标识，并具有两类来源之一：

- `Proven`：由经过验证的编译器分析推导；
- `TrustedContract`：从支配当前位置的 unsafe 函数入口导入。

证明在事实、指令、基本块和效果摘要之上形成依赖 DAG。变换和诊断输出始终保留
来源区别。

## 分析

### 标量与路径事实

标量抽象域组合 signed/unsigned 区间、仿射关系、congruence 和内部 known-bits。
分支边细化路径敏感事实。自然循环 phi 使用确定的 widening 保证终止，再执行固
定次数 narrowing 恢复精度。

在分析证明不会失败前，checked 算术始终带潜在失败效果。unchecked 整数算术使
用规定的模运算语义，不能继承会被环绕破坏的数学整数结论。

分析上限由 KIR 大小和固定配置决定，绝不使用墙钟时间。达到上限会得到 `unknown`
或保守摘要。

### 别名与内存事实

区域身份、符号 sub-slice 区间、`noalias`、访问宽度和对齐进入同一个共享别名查
询服务。Memory SSA、load forwarding、死存储消除、LICM 和后端 metadata 都使
用该服务，不得实现 pass 私有别名规则。

### 跨过程效果

编译器在调用图强连通分量上自底向上求解效果摘要。摘要记录映射到参数的读写、
runtime print、潜在 checked 失败和 unsafe 调用。递归分量使用单调迭代求不动
点。未知或超过预算的函数退化为 `readwrite all + may_fail + runtime_effect`。

## 携带证明的变换

pass 不能静默删除或弱化 bounds/overflow guard。它必须提交带 `ProofId` 的变换，
标明使用的支配范围、控制、slice 长度、别名、对齐、效果和契约事实。独立 KIR
verifier 在每个 pass 后，依据当前 CFG、标量 SSA、Memory SSA 和效果顺序检查证
明。

CFG 修改、内联和循环修改通过显式分析保留声明使事实和证明失效。过期或无效证
明属于编译器内部错误：停止编译且不提交任何制品。编译器不得通过生成未验证机
器码恢复。

## 优化级别与 0.11 范围

O0 构造并验证 KIR，但不执行可选优化。O1 增加 CFG canonicalization、稀疏条件
常量与范围传播、携带证明的冗余检查消除、死代码消除和清理。O2 增加效果感知内
联、全局值编号、基于 Memory SSA 的 load forwarding、死存储消除、第二轮传播/
检查消除和清理。O3 增加自然循环与归纳分析、效果/别名感知 LICM、归纳简化、最
后一轮传播/检查消除和清理。

每个命名 pass 都必须确定，并在其后执行 KIR 验证。一个选定级别控制所有后端共
享的 KIR 管线以及后续 Native LLVM 优化级别。

CK 0.11 明确不包含自动 SIMD、循环展开、函数专用化、PGO、Auto-Tuning、
fast-math、局部假设或永久双管线。既有严格浮点、checked 错误、runtime 效果和
ABI 行为保持不变。

## 后端事实映射

只有经过验证的 KIR 事实才能加强后端 IR。

Native LLVM lowering 使用经过审查的白名单，包括 `noalias`、`readonly`、
`writeonly`、适用的 `memory(...)` 效果、alignment、`alias.scope`/`noalias`
metadata、整数 range 和已证明的 `nuw`/`nsw`。只有当经过验证的事实没有更直接
表示且假设确实有用时才生成 `llvm.assume`。每项加强都在编译器审计映射中保留
`FactId` 或 `ProofId`。lowering 后审计会拒绝没有可接受 KIR 来源的 LLVM 属性或
flag。

C 后端消费相同的优化 KIR，并可以通过标准 `restrict` 和条件定义的编译器对齐
提示表达等价事实；可移植 fallback 仍是有效 C。WebAssembly 后端消费 KIR 检查
消除和已证明访问对齐，但不虚构 checked ABI 或不支持的别名 metadata。任何后端
都不能推导比 KIR 更强的源码契约。

LLVM 继续负责机器级 canonicalization、指令选择、寄存器分配、调度、目标合法
化及其自身合法的后续优化，但不再是 CK 唯一的事实发现层。

## 检查、诊断与 sanitizer

0.11 CLI 增加确定的检查界面：

- `ckc emit-kir`；
- `--print-facts`；
- `--print-effect-summaries`；
- `--explain-optimization`；
- `--sanitize-contracts`。

优化解释应说明每个被删除或保留的检查、使用的事实和证明、是否使用可信契约，
以及没有合法变换时的保守原因。输出不得包含绝对路径、地址、时间戳或无序 map
迭代结果。

契约 sanitizer 仅适用于 `run` 和 executable build。它在 unsafe 调用及导出入
口边界插入可动态验证的 `requires` 检查。effects 上限继续在编译期检查。违反时
必须在 stderr 精确输出 `CKR0007: unsafe contract violation` 加 LF，并以状态
246 退出。sanitizer 行为是调试设施，不是正常语言语义、正式库 ABI 或 benchmark
证据。0.11 将状态 246 保留给该 sanitizer 失败；生产 runtime 状态仍由正常
overflow、bounds、输出和子进程契约决定。

本规范新增源码诊断：

| Code | 含义 |
| --- | --- |
| `CK2014` | 非法 unsafe 函数、contract 位置、unsafe 块或 unsafe 调用边界。 |
| `CK2015` | 非法、类型错误、不受支持或不可判定的契约表达式或谓词。 |
| `CK2016` | 声明的效果上限无法覆盖静态推导的函数效果。 |

KIR verifier 和后端事实审计失败是编译器错误，不是 CK 源码诊断。

## 验收契约

满足以下全部条件前，CK 0.11 不算完成：

1. 所有既有 0.10 语义、ABI、checked 首错、runtime print、CLI、制品和分发测试
   在没有 ignored test 或降低门槛的情况下继续通过。
2. 所有正式及兼容 fixtures 在 O0–O3 和每个受支持 checked/unchecked 组合上通
   过 C、WebAssembly 和 Native 差分测试。
3. 契约测试覆盖合法语法、每种非法边界和谓词、外部导出注释、unsafe 调用、即
   时 UB 模型及 sanitizer 正反执行。
4. KIR mutation tests 能证明 verifier 拒绝缺失支配、错误标量或 memory phi、
   错误 alias partition、过期事实、无效 ProofId，以及潜在失败或 runtime 效果
   重排。
5. 固定种子的生成 Kernel 对比未优化和优化后的可观察行为；契约生成用例只使用
   满足声明定义域的输入。
6. LLVM 事实审计在全部六个发布目标上通过，并拒绝故意注入且没有 KIR 来源的属
   性。
7. 典型可证明循环在 O2/O3 热循环内没有冗余 bounds guard，并通过 KIR 和后端
   IR 结构检查确认。
8. 既有 Native 对固定 Clang O3 门禁继续保持至少 95% 几何平均吞吐，且单个
   Kernel 不得慢于 10%。
9. 在受控 worker 上相对固定且已验收的 0.10 编译器基线，0.11 runtime 吞吐几何
   平均回退不得超过 3%，单项不得超过 8%。benchmark manifest 必须记录准确的
   0.10 源码摘要和编译器身份，不能依赖移动分支。
10. 对检查已被完全证明冗余的验收循环套件，checked 几何平均吞吐至少达到对应
    unchecked 执行的 97%。
11. KIR 分析与优化耗时中位数最多为固定 0.10 MIR 优化耗时的 2 倍，任一验收用
    例不得超过 3 倍。预算 fallback 必须保持语义并报告保守原因。

运行时比较固定源码、编译器身份、target、CPU policy、安全模式、严格浮点行为、
harness、warm-up、重复次数和统计规则。修改门槛或语料属于需要审查的契约变更。

## 0.11 之后的性能计划

以下工作需要独立的版本化规范，不能扩大 0.11 实施计划：

- 0.12：循环规范化和依赖合法性、收益模型、loop/SLP SIMD、受控展开/版本化及
  事实驱动函数专用化；
- 0.13：baseline+feature CPU 多版本、稳定 profile schema、profile 插桩/使用
  和 PGO；
- 0.14：有预算、可复现且可缓存的离线 Auto-Tuning，其缓存键覆盖编译器与 ABI
  身份、Kernel 与契约摘要、目标 CPU、profile、候选空间和测量策略。

对适合向量化的 Kernel，最终门禁固定经过审计的手写 C/Rust+SIMD 源码，并要求
CK 几何平均吞吐至少达到该 oracle 的 95%，每个 Kernel 至少达到 90%。另设一套
CK 契约能暴露而固定通用源码无法获得领域约束的测试，要求其在相同可观察行为和
有效输入定义域及安全语义下，几何平均至少超过固定通用 Clang/Rust O3 结果 5%。
手写 SIMD oracle 获得其源码语言能够表达的全部等价前置条件。通用默认 oracle
是独立且经过审计的源码，不是从优化后 KIR 生成的 C；它不得包含隐藏 UB，并且只
编码该源码实际具有的事实。Auto-Tuning 必须在固定预算内选择合法候选，保留
baseline fallback，且不得让运行时探索成为隐式生产依赖。
