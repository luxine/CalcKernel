# 命名规范

CalcKernel 在源码、生成产物、文档、测试和发布包中使用统一的语言命名。

## 标准名称

- 语言名：CK / CalcKernel
- 编译器命令：`ckc`
- 源码文件后缀：`.ck`
- C ABI 前缀：`CK_`

项目不支持其他编译器命令别名，也不支持其他源码后缀别名。

## 规则

- 使用 CK / CalcKernel 表示语言和项目。
- 所有 CLI 示例都使用 `ckc`。
- 所有源码文件都使用 `.ck`。
- 生成的 C ABI 使用 `CK_API`、`CK_BUILD_DLL`、`CK_Status`、`CK_OK` 和 `CK_ERR_*`。
- 示例文件放在 `examples/*.ck`。
- 测试和 snapshot 必须与 `ckc` 和 `.ck` 保持一致。
- 除非未来用户明确要求，否则不要添加兼容别名。

## 示例

```sh
ckc check examples/pricing.ck
```

```sh
ckc emit-c examples/pricing.ck \
  --out build/pricing.c \
  --header build/pricing.h
```

```sh
ckc emit-wasm examples/pricing.ck \
  --out build/pricing.wasm
```

```sh
ckc emit-llvm examples/pricing.ck \
  --out build/pricing.ll
```
