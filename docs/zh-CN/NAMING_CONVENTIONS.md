# 命名规范

IntKernel 在源码、生成产物、文档、测试和发布包中使用统一的语言命名。

## 标准名称

- 语言名：IK / IntKernel
- 编译器命令：`ikc`
- 源码文件后缀：`.ik`

项目不支持其他编译器命令别名，也不支持其他源码后缀别名。

## 规则

- 使用 IK / IntKernel 表示语言和项目。
- 所有 CLI 示例都使用 `ikc`。
- 所有源码文件都使用 `.ik`。
- 示例文件放在 `examples/*.ik`。
- 测试和 snapshot 必须与 `ikc` 和 `.ik` 保持一致。
- 除非未来用户明确要求，否则不要添加兼容别名。

## 示例

```sh
ikc check examples/pricing.ik
```

```sh
ikc emit-c examples/pricing.ik \
  --out build/pricing.c \
  --header build/pricing.h
```

```sh
ikc emit-wasm examples/pricing.ik \
  --out build/pricing.wasm
```

```sh
ikc emit-llvm examples/pricing.ik \
  --out build/pricing.ll
```
