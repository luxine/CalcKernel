# 阶段 09 任务：0.12.0 release identity、current docs 与兼容契约

## 目标

在阶段 08 功能/语义门禁通过后、最终性能门禁前，把 package/compiler/current documentation
更新为 0.12.0，冻结 source/CLI/KIR/cache/private bridge/public ABI 契约，形成真实性能可使用
的候选身份；不创建 tag/Release，不合并 main。

## 仓库落点

- `Cargo.toml`/`Cargo.lock`、repository/release contract tests。
- `README.md`/`README.zh-CN.md`、`docs/**` 对应英文/中文 current docs。
- `CHANGELOG.md`、compatibility fixtures/manifest、release checklist/workflow identity。
- 清理临时/历史阶段性内容只按现有规范：本次用户明确要求的 `specs/0.12/implementation` 与
  `review` 保留作为候选审查输入；不得把它们误放进 current `docs/`。

## TDD 顺序

1. 先把 repository/release/docs/compatibility contract tests 改为期望当前 0.12.0、KIR v2、
   cache schema 3、bridge ABI 3、Native ABI 1、Runtime ABI 2，观察 version/docs/manifest RED。
2. 更新 `Cargo.toml`/`Cargo.lock`、compiler verbose identity、release workflow grep 与
   compatibility manifest；不创建 tag/Release。
3. 更新 current docs 双语：architecture、optimizer/vector guarantees、CLI consumer/cpu、cache、
   performance schema 7、build/toolchain、release/support boundary；删除仍把 0.11 当 current 的文本。
4. 更新 changelog 为用户可见行为，不宣称 0.13 PGO/multiversion 或 0.14 autotune 已实现。
5. 跑文档 link/anchor/mirror contract、compatibility v0.11 fixtures、headers/symbol/ABI audits。
6. 生成 release binary，验证 `--version --verbose`、help、licenses、run/build smoke、零运行时依赖
   审计；六平台 archive 仍由 release workflow 在未来 tag 时生成，本任务不发布。
7. 形成 release-identity 检查点并通过本地完整回归。性能 schema 与最终 exact-SHA 十作业
   留到阶段 10；本阶段不以旧 0.11 性能报告代签。

## 实现判定

- Source/public ABI 保持兼容；KIR/private bridge/cache 按设计有意不兼容且 fail-closed。
- 当前 docs 只描述实际已通过的 0.12；设计/计划不冒充 current language reference。
- Tag `v0.12.0` 和 GitHub Release 明确不存在；main 不变。
