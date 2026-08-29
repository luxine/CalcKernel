# 阶段 01 任务：unsafe 与契约前端

## 目标

在不改变既有 safe CK 语义的前提下，解析、类型检查并保存 `unsafe fn`、unsafe 语句
块、封闭 contract DSL 和 memory effect ceiling。此阶段不做 CK2016 的跨过程 ceiling
验证；它在阶段 04 由共享 effect summary 完成。

## 仓库落点

- `src/frontend/lexer.rs`：新增保留字 token；不得把 contract predicate 当普通函数。
- `src/frontend/ast.rs`：新增 `ContractDeclaration`、typed contract expression/predicate、
  memory effect target，以及 `Statement::Unsafe`。
- `src/frontend/parser.rs`：实现 `[export] [unsafe] fn ... contract {...} {...}` 与
  `unsafe {...}`；错误恢复必须停在 clause 分号或 contract/body 右花括号。
- `src/frontend/typeck.rs`：保存函数 unsafe 属性与 contract；检查 CK2014/15、unsafe
  call context、main 禁令、slice/ptr predicate 类型、affine 封闭性与 effect target。
- `src/frontend/diagnostics.rs`：稳定增加 CK2014–CK2016；本阶段只产生 CK2014/15。
- `tests/frontend/contracts.rs` 并登记到 `tests/frontend.rs`。

## TDD 顺序

1. 先写 lexer/parser red tests，覆盖完整示例、modifier 顺序、多个 requires、effects
   none/list、unsafe block、每个新 token 的 UTF-16 span。观察原 lexer 把关键字当
   identifier 或 parser 拒绝的失败。
2. 最小实现 token、AST 与 parser；运行对应 tests 到 green 后才重构 parser helper。
3. 写 CK2014 red tests：safe fn contract/effects、unsafe fn 无 requires、unsafe call 不在
   block、unsafe main、孤立 contract、错误 modifier 顺序；实现 unsafe-depth 检查。
4. 写 CK2015 red tests：非 bool requires、普通 call、析取/否定、memory load、非 affine
   乘法、混合 signed/unsigned、错误 noalias/aligned/multiple_of、非 slice effect target、
   重复/冲突 effect target、越界 alignment 常量。
5. 写 normalized checked metadata tests，证明 contract integer expression 在完成普通类
   型检查后标记为 mathematical，而不是生成普通可执行 AST call。
6. 跑全部 frontend tests，确认既有 safe fixtures AST 与诊断 bytes 不漂移。

## 实现判定

- `unsafe` 只建立调用许可边界，不抑制其他诊断。
- 每个 call expression 无论嵌套在 let/return/argument 何处，都按所在 statement 的
  unsafe-depth 判断。
- `main` 一旦带 unsafe/contract/effects 即 CK2014 且不形成 entry。
- contract 常量必须先满足对应 CK integer literal/type 规则；规范化 affine coefficient
  与 term 顺序必须确定。
- effect list 规范化为每个 slice parameter 一个 `None/Read/Write/ReadWrite` lattice 值，
  但不在本阶段猜测函数体效果。

## 明确不做

不构造 KIR，不导入 TrustedContract，不插入 sanitizer，不发出 header 注释，不实现
CK2016，不改变 runtime 或 backend。
