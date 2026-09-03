# 阶段 12 验收：Profile Runtime 可移植性

## 本地必须通过

- [ ] `cargo test --locked --test contracts profile_runtime_ -- --nocapture`
- [ ] `cargo test --all-features --locked --test native profile_generation_ -- --nocapture`
- [ ] `cargo test --all-features --locked --test native profile_runtime_ -- --nocapture`
- [ ] `cargo test --all-features --locked --test native artifacts -- --nocapture`

## 六 host 必须通过

- [ ] MSVC x86-64 与 AArch64 均以 C 模式编译 collector，且不请求 C11 stdatomic。
- [ ] Linux x86-64/AArch64 与 Darwin x86-64/AArch64 均产生、flush、关闭、
  no-follow reopen 并解析唯一 completed shard。
- [ ] Darwin runtime object 不包含未冻结 `_fstat$INODE64` 依赖。
- [ ] 每个平台均验证 create-new、no-replace、file sync、directory sync 和
  identity mismatch fail-closed；任何 required capability 不得 skip。

## 结构断言

- [ ] 所有 collector 原子访问只经过 `ckc_profile_atomic.h`。
- [ ] MSVC Interlocked 操作不弱于原 acquire/release/relaxed 语义；Unix 64 位
  原子在编译期要求 always-lock-free。
- [ ] 六个发布 failure step 均有注入测试，失败后没有 completed partial file。
- [ ] Runtime ABI 2、CKPROF01、CKPART01 与公开状态码没有变化。
- [ ] provenance/component digest 包含新 header，release artifact 不新增依赖。

## 完成证据

命令、test count、host/target、runtime object symbol/dependency 摘要写入
`target/acceptance/v0.14/stage-12/`，不提交生成物。
