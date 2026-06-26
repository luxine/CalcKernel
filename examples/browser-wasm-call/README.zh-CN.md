# Browser WASM Pricing 示例

[English](README.md)

这个示例在浏览器中运行从 `examples/pricing.ck` 生成的 WebAssembly module。
它是纯 HTML 和 JavaScript：不使用框架、不使用 bundler，也没有额外依赖。

## 生成 WASM

从仓库根目录运行：

```sh
pnpm build
pnpm ckc emit-wasm examples/pricing.ck --out examples/browser-wasm-call/pricing.wasm
```

示例期望 `pricing.wasm` 与 `index.html` 和 `index.js` 位于同一目录：

```text
examples/browser-wasm-call/
  index.html
  index.js
  pricing.wasm
```

## 本地启动

浏览器通常不能可靠地从 `file://` 加载 WASM。请启动本地 HTTP server：

```sh
cd examples/browser-wasm-call
python3 -m http.server 8000
```

然后打开：

```text
http://localhost:8000/index.html
```

点击 **Run pricing wasm**。页面会显示 `calc_items` 返回码和计算后的 output buffer。

## 浏览器 ABI 说明

- `i64` / `u64` 使用 JavaScript `BigInt`。
- `ptr<T>` 是 exported WASM memory 中的数字 offset。
- 示例使用 `DataView` 写入 `Item` struct，并读取 output buffer。
- WebAssembly memory 是 little-endian，所以 `DataView` 读写都传入
  `true` 作为 little-endian 参数。
- WASM v1 不做 bounds check。
- WASM v1 不实现 checked overflow。

调用方必须选择有效 memory offset，并为输入 `Item` 数组和输出 `i64` buffer
分配足够空间。

这个浏览器示例刻意使用 `DataView`，因为它演示的是 mixed-width pricing `Item`
ABI。它应被视为 ABI/debug fallback，而不是高吞吐 pricing 推荐路径。大批量
pricing 场景应优先使用 SoA resident memory、`BigInt64Array` bulk copy 和
output view，详见 `docs/wasm-interop.md`。
