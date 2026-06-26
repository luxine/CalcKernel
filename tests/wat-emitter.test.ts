import { describe, expect, it } from "vitest";
import { emitWatModule } from "../src/backend/wasm/wat-emitter.js";
import { toWasmIdentifier } from "../src/backend/wasm/wasm-names.js";

describe("WAT emitter", () => {
  it("emits an empty module with exported memory", () => {
    expect(emitWatModule()).toBe(`(module
  (memory (export "memory") 1)
  (global (export "__ck_heap_base") i32 (i32.const 0))
)
`);
  });

  it("emits an exported add_i64 function skeleton", () => {
    expect(
      emitWatModule({
        functions: [
          {
            name: "add_i64",
            exportName: "add_i64",
            params: [
              { name: "a", type: "i64" },
              { name: "b", type: "i64" }
            ],
            result: "i64"
          }
        ]
      })
    ).toBe(`(module
  (memory (export "memory") 1)
  (global (export "__ck_heap_base") i32 (i32.const 0))

  (func $add_i64 (export "add_i64")
    (param $a i64)
    (param $b i64)
    (result i64)
  )
)
`);
  });

  it("emits function locals with stable indentation", () => {
    expect(
      emitWatModule({
        functions: [
          {
            name: "with_locals",
            params: [{ name: "a", type: "i64" }],
            result: "i64",
            locals: [
              { name: "ik_tmp0", type: "i64" },
              { name: "flag", type: "i32" }
            ],
            body: ["local.get $a", "local.set $ik_tmp0"]
          }
        ]
      })
    ).toBe(`(module
  (memory (export "memory") 1)
  (global (export "__ck_heap_base") i32 (i32.const 0))

  (func $with_locals
    (param $a i64)
    (result i64)
    (local $ik_tmp0 i64)
    (local $flag i32)
    local.get $a
    local.set $ik_tmp0
  )
)
`);
  });

  it("escapes unsafe WAT identifiers and export strings", () => {
    expect(toWasmIdentifier("items[i].price")).toBe("$items_x5b_i_x5d__x2e_price");

    expect(
      emitWatModule({
        functions: [
          {
            name: "items[i].price",
            exportName: "quote\"slash\\",
            params: [{ name: "arg-0", type: "i32" }]
          }
        ]
      })
    ).toBe(`(module
  (memory (export "memory") 1)
  (global (export "__ck_heap_base") i32 (i32.const 0))

  (func $items_x5b_i_x5d__x2e_price (export "quote\\22slash\\5c")
    (param $arg_x2d_0 i32)
  )
)
`);
  });
});
