use std::process::Command;

use calckernel::{
    MirBlock, MirFunction, MirInstruction, MirLocal, MirModule, MirParam, MirPlace,
    MirPrimitiveTypeName, MirStruct, MirStructField, MirTerminator, MirType, MirValue, SourceFile,
    check, lower_to_mir, print_mir_module, validate_mir_module,
};

use super::support::fixtures;
use super::support::oracle::{typescript_cli, typescript_root};

fn lower_and_print(source_text: &str) -> String {
    let checked = check(&SourceFile::new("test.ck", source_text));
    assert_eq!(checked.diagnostics, []);
    let mir = lower_to_mir(&checked.checked_program).expect("MIR lowering should succeed");
    assert_eq!(validate_mir_module(&mir).errors, []);
    print_mir_module(&mir)
}

#[test]
fn mir_should_lower_scalar_straight_line_functions() {
    assert_eq!(
        lower_and_print(
            r#"
        export fn add_i64(a: i64, b: i64) -> i64 {
          let x: i64 = a + b;
          return x;
        }

        export fn assign_i64(a: i64, b: i64) -> i64 {
          let x: i64 = a;
          x = b - 1;
          return x;
        }
      "#
        ),
        "export fn add_i64(a: i64, b: i64) -> i64 {
  local x: i64

bb0:
  %t0: i64 = add a, b
  x: i64 = move %t0
  return x
}

export fn assign_i64(a: i64, b: i64) -> i64 {
  local x: i64

bb0:
  x: i64 = move a
  %t0: i64 = const_int 1
  %t1: i64 = sub b, %t0
  x: i64 = move %t1
  return x
}
"
    );
}

#[test]
fn mir_should_lower_control_flow() {
    assert_eq!(
        lower_and_print(
            r#"
        export fn sum_to_n(n: i64) -> i64 {
          let i: i64 = 0;
          let sum: i64 = 0;

          while i < n {
            sum = sum + i;
            i = i + 1;
          }

          return sum;
        }
      "#
        ),
        "export fn sum_to_n(n: i64) -> i64 {
  local i: i64
  local sum: i64

bb0:
  %t0: i64 = const_int 0
  i: i64 = move %t0
  %t1: i64 = const_int 0
  sum: i64 = move %t1
  jump bb1

bb1:
  %t2: bool = lt i, n
  branch %t2, bb2, bb3

bb2:
  %t3: i64 = add sum, i
  sum: i64 = move %t3
  %t4: i64 = const_int 1
  %t5: i64 = add i, %t4
  i: i64 = move %t5
  jump bb1

bb3:
  return sum
}
"
    );
}

#[test]
fn mir_should_lower_places() {
    assert_eq!(
        lower_and_print(
            r#"
          struct Quote {
            price: f64;
            qty: i64;
          }

          export fn update(items: ptr<Quote>, out: ptr<f64>, i: i32) -> f64 {
            let price: f64 = items[i].price;
            out[i] = price;
            return out[i];
          }
        "#
        ),
        "struct Quote {
  price: f64
  qty: i64
}

export fn update(items: ptr<Quote>, out: ptr<f64>, i: i32) -> f64 {
  local price: f64

bb0:
  %t0: f64 = load field(index(items, i), price)
  price: f64 = move %t0
  store index(out, i), price
  %t1: f64 = load index(out, i)
  return %t1
}
"
    );
}

#[test]
fn mir_should_lower_short_circuit_logical_operators() {
    assert_eq!(
        lower_and_print(
            r#"
        export fn and_short_circuit(a: i64, b: i64) -> bool {
          return a != 0 && b / a > 1;
        }
      "#
        ),
        "export fn and_short_circuit(a: i64, b: i64) -> bool {
  local ik_sc0: bool

bb0:
  %t0: i64 = const_int 0
  %t1: bool = ne a, %t0
  branch %t1, bb1, bb2

bb1:
  %t2: i64 = div b, a
  %t3: i64 = const_int 1
  %t4: bool = gt %t2, %t3
  ik_sc0: bool = move %t4
  jump bb3

bb2:
  ik_sc0: bool = move false
  jump bb3

bb3:
  return ik_sc0
}
"
    );
}

#[test]
fn mir_should_lower_break_to_innermost_loop_exit() {
    let text = lower_and_print(
        r#"
      export fn stop_at_three(n: u32) -> u32 {
        let i: u32 = 0;
        while i < n {
          if i == 3 {
            break;
          }
          i = i + 1;
        }
        return i;
      }
    "#,
    );

    assert!(text.contains("bb4:\n  jump bb3\n"), "{text}");
}

#[test]
fn mir_should_lower_continue_to_innermost_loop_condition() {
    let text = lower_and_print(
        r#"
      export fn skip_three(n: u32) -> u32 {
        let i: u32 = 0;
        let total: u32 = 0;
        while i < n {
          i = i + 1;
          if i == 3 {
            continue;
          }
          total = total + i;
        }
        return total;
      }
    "#,
    );

    assert!(text.contains("bb4:\n  jump bb1\n"), "{text}");
}

#[test]
fn mir_should_lower_nested_loop_control_to_distinct_targets() {
    let text = lower_and_print(
        r#"
      export fn nested(n: u32) -> u32 {
        let outer: u32 = 0;
        let inner: u32 = 0;
        while outer < n {
          while inner < n {
            break;
          }
          continue;
        }
        return outer;
      }
    "#,
    );

    assert!(text.contains("bb5:\n  jump bb6\n"), "{text}");
    assert!(text.contains("bb6:\n  jump bb1\n"), "{text}");
}

#[test]
fn mir_should_print_targetless_void_calls_and_valueless_returns() {
    let text = lower_and_print(
        r#"
      fn touch(out: ptr<i32>) -> void {
        out[0] = 1;
        return;
      }

      export fn run(out: ptr<i32>) -> void {
        touch(out);
      }
    "#,
    );

    assert!(text.contains("fn touch(out: ptr<i32>) -> void"), "{text}");
    assert!(text.contains("  call touch(out)\n"), "{text}");
    assert!(text.matches("  return\n").count() >= 2, "{text}");
    assert!(!text.contains(": void ="), "{text}");
}

#[test]
fn mir_should_insert_return_none_for_void_fallthrough() {
    let checked = check(&SourceFile::new("test.ck", "fn no_op() -> void {}"));
    assert_eq!(checked.diagnostics, []);
    let module = lower_to_mir(&checked.checked_program).expect("lower void fallthrough");
    assert!(matches!(
        module.functions[0].blocks[0].terminator,
        MirTerminator::Return { value: None }
    ));
}

#[test]
fn mir_validator_should_reject_void_values_and_call_return_mismatches() {
    let i32_type = MirType::Primitive(MirPrimitiveTypeName::I32);
    let void_value = MirValue::Temp {
        name: "void_temp".to_string(),
        type_node: MirType::Void,
    };
    let i32_value = MirValue::Temp {
        name: "value".to_string(),
        type_node: i32_type.clone(),
    };
    let module = MirModule {
        structs: vec![MirStruct {
            name: "Bad".to_string(),
            fields: vec![MirStructField {
                name: "field".to_string(),
                type_node: MirType::Void,
            }],
        }],
        functions: vec![
            MirFunction {
                name: "procedure".to_string(),
                exported: false,
                params: vec![MirParam {
                    name: "bad".to_string(),
                    type_node: MirType::Void,
                }],
                return_type: MirType::Void,
                locals: vec![MirLocal {
                    name: "also_bad".to_string(),
                    type_node: MirType::Void,
                }],
                blocks: vec![MirBlock {
                    label: "bb0".to_string(),
                    instructions: vec![
                        MirInstruction::Store {
                            place: MirPlace::Local {
                                name: "also_bad".to_string(),
                                type_node: MirType::Void,
                            },
                            value: MirValue::ConstInt {
                                text: "0".to_string(),
                                type_node: MirType::Void,
                            },
                        },
                        MirInstruction::Call {
                            target: Some(i32_value.clone()),
                            function_name: "procedure".to_string(),
                            args: vec![],
                        },
                    ],
                    terminator: MirTerminator::Return {
                        value: Some(void_value),
                    },
                }],
            },
            MirFunction {
                name: "value".to_string(),
                exported: false,
                params: vec![],
                return_type: i32_type,
                locals: vec![],
                blocks: vec![MirBlock {
                    label: "bb0".to_string(),
                    instructions: vec![MirInstruction::Call {
                        target: None,
                        function_name: "value".to_string(),
                        args: vec![],
                    }],
                    terminator: MirTerminator::Return { value: None },
                }],
            },
        ],
    };

    let messages = validate_mir_module(&module)
        .errors
        .into_iter()
        .map(|error| error.message)
        .collect::<Vec<_>>();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Void parameter"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Void local"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Void field"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Void MIR value"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("Void MIR place"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("void call cannot have a target"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("non-void call requires a target"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("void return cannot have a value"))
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("non-void return requires a value"))
    );
}

#[test]
fn mir_validator_should_accept_void_control_flow_with_all_blocks_terminated() {
    let checked = check(&SourceFile::new(
        "test.ck",
        r#"
      fn no_op() -> void {}
      export fn control(stop: bool) -> void {
        while true {
          if stop {
            return;
          }
          no_op();
          break;
        }
      }
    "#,
    ));
    assert_eq!(checked.diagnostics, []);
    let module = lower_to_mir(&checked.checked_program).expect("lower void control flow");
    assert_eq!(validate_mir_module(&module).errors, []);
    assert!(
        module
            .functions
            .iter()
            .all(|function| !function.blocks.is_empty())
    );
}

#[test]
fn mir_should_print_make_slice_projections_index_and_subslice() {
    let text = lower_and_print(
        r#"
      fn cut(data: ptr<i64>, len: u32, start: u32, end: u32) -> slice<i64> {
        let items: slice<i64> = slice(data, len);
        let raw: ptr<i64> = items.data;
        let count: u32 = items.len;
        let value: i64 = items[start];
        return items[start..end];
      }
    "#,
    );

    assert!(text.contains("make_slice data, len"), "{text}");
    assert!(text.contains("slice_data items"), "{text}");
    assert!(text.contains("slice_len items"), "{text}");
    assert!(text.contains("load slice_index(items, start)"), "{text}");
    assert!(text.contains("subslice items, start, end"), "{text}");
    assert!(text.contains("-> slice<i64>"), "{text}");
}

#[test]
fn mir_should_copy_slice_locals_fields_arguments_and_internal_returns() {
    let text = lower_and_print(
        r#"
      struct Holder { values: slice<i32>; }
      fn identity(values: slice<i32>) -> slice<i32> { return values; }
      fn copy(src: ptr<Holder>, dst: ptr<Holder>) -> slice<i32> {
        let local: slice<i32> = src[0].values;
        dst[0].values = identity(local);
        return dst[0].values;
      }
    "#,
    );

    assert!(text.contains("values: slice<i32>"), "{text}");
    assert!(text.contains("local local: slice<i32>"), "{text}");
    assert!(text.contains("load field(index(src,"), "{text}");
    assert!(text.contains("call identity(local)"), "{text}");
    assert!(text.contains("store field(index(dst,"), "{text}");
    assert!(text.contains("return %"), "{text}");
}

#[test]
fn mir_should_evaluate_slice_then_index_or_range_operands_once_in_order() {
    let text = lower_and_print(
        r#"
      fn make(data: ptr<i32>, len: u32) -> slice<i32> { return slice(data, len); }
      fn next(state: ptr<u32>) -> u32 {
        let value: u32 = state[0];
        state[0] = value + 1;
        return value;
      }
      fn ordered(data: ptr<i32>, len: u32, state: ptr<u32>) -> slice<i32> {
        return make(data, len)[next(state)..next(state)];
      }
    "#,
    );

    let ordered = text.split("fn ordered").nth(1).expect("ordered function");
    let make = ordered.find("call make(data, len)").expect("slice call");
    let first = ordered.find("call next(state)").expect("first endpoint");
    let second = ordered[first + 1..]
        .find("call next(state)")
        .map(|offset| first + 1 + offset)
        .expect("second endpoint");
    let subslice = ordered.find("subslice").expect("subslice");
    assert!(
        make < first && first < second && second < subslice,
        "{ordered}"
    );
    assert_eq!(ordered.matches("call make(data, len)").count(), 1);
    assert_eq!(ordered.matches("call next(state)").count(), 2);
}

#[test]
fn mir_validator_should_reject_each_malformed_slice_operation() {
    let i32_type = MirType::Primitive(MirPrimitiveTypeName::I32);
    let i64_type = MirType::Primitive(MirPrimitiveTypeName::I64);
    let u32_type = MirType::Primitive(MirPrimitiveTypeName::U32);
    let slice_i32 = MirType::Slice(Box::new(i32_type.clone()));
    let slice_i64 = MirType::Slice(Box::new(i64_type.clone()));
    let temp = |name: &str, type_node: MirType| MirValue::Temp {
        name: name.to_string(),
        type_node,
    };
    let param = |name: &str, type_node: MirType| MirValue::Param {
        name: name.to_string(),
        type_node,
    };
    let instructions = vec![
        MirInstruction::MakeSlice {
            target: temp("made", slice_i32.clone()),
            data: param("wrong_data", i64_type.clone()),
            len: param("wrong_len", i32_type.clone()),
        },
        MirInstruction::SliceData {
            target: temp("data", i64_type.clone()),
            slice: param("wrong_data", i64_type.clone()),
        },
        MirInstruction::SliceLen {
            target: temp("len", i64_type.clone()),
            slice: param("wrong_data", i64_type.clone()),
        },
        MirInstruction::Subslice {
            target: temp("sub", slice_i64.clone()),
            slice: param("items", slice_i32.clone()),
            start: param("wrong_len", i32_type.clone()),
            end: param("wrong_data", i64_type.clone()),
        },
        MirInstruction::Load {
            target: temp("loaded", i64_type.clone()),
            place: MirPlace::SliceIndex {
                slice: param("items", slice_i32.clone()),
                index: param("wrong_len", i32_type.clone()),
                type_node: i64_type.clone(),
            },
        },
    ];
    let module = MirModule {
        structs: vec![],
        functions: vec![MirFunction {
            name: "bad".to_string(),
            exported: false,
            params: vec![
                MirParam {
                    name: "wrong_data".to_string(),
                    type_node: i64_type,
                },
                MirParam {
                    name: "wrong_len".to_string(),
                    type_node: i32_type,
                },
                MirParam {
                    name: "items".to_string(),
                    type_node: slice_i32,
                },
                MirParam {
                    name: "ok_len".to_string(),
                    type_node: u32_type,
                },
            ],
            return_type: MirType::Void,
            locals: vec![],
            blocks: vec![MirBlock {
                label: "bb0".to_string(),
                instructions,
                terminator: MirTerminator::Return { value: None },
            }],
        }],
    };

    let messages = validate_mir_module(&module)
        .errors
        .into_iter()
        .map(|error| error.message)
        .collect::<Vec<_>>();
    for operation in [
        "MakeSlice",
        "SliceData",
        "SliceLen",
        "Subslice",
        "SliceIndex",
    ] {
        assert!(
            messages.iter().any(|message| message.contains(operation)),
            "missing {operation}: {messages:#?}"
        );
    }
}

#[test]
fn mir_validator_should_reject_void_or_direct_slice_elements_and_exported_returns() {
    let i32_type = MirType::Primitive(MirPrimitiveTypeName::I32);
    let slice_i32 = MirType::Slice(Box::new(i32_type));
    let slice_i64 = MirType::Slice(Box::new(MirType::Primitive(MirPrimitiveTypeName::I64)));
    let module = MirModule {
        structs: vec![
            MirStruct {
                name: "VoidElement".to_string(),
                fields: vec![MirStructField {
                    name: "bad".to_string(),
                    type_node: MirType::Slice(Box::new(MirType::Void)),
                }],
            },
            MirStruct {
                name: "Nested".to_string(),
                fields: vec![MirStructField {
                    name: "bad".to_string(),
                    type_node: MirType::Slice(Box::new(slice_i32.clone())),
                }],
            },
        ],
        functions: vec![
            MirFunction {
                name: "exported".to_string(),
                exported: true,
                params: vec![],
                return_type: slice_i32.clone(),
                locals: vec![],
                blocks: vec![MirBlock {
                    label: "bb0".to_string(),
                    instructions: vec![],
                    terminator: MirTerminator::Return {
                        value: Some(MirValue::ConstInt {
                            text: "0".to_string(),
                            type_node: slice_i32.clone(),
                        }),
                    },
                }],
            },
            MirFunction {
                name: "identity".to_string(),
                exported: false,
                params: vec![MirParam {
                    name: "items".to_string(),
                    type_node: slice_i32.clone(),
                }],
                return_type: slice_i32.clone(),
                locals: vec![],
                blocks: vec![MirBlock {
                    label: "bb0".to_string(),
                    instructions: vec![],
                    terminator: MirTerminator::Return {
                        value: Some(MirValue::Param {
                            name: "items".to_string(),
                            type_node: slice_i32.clone(),
                        }),
                    },
                }],
            },
            MirFunction {
                name: "bad_call".to_string(),
                exported: false,
                params: vec![MirParam {
                    name: "wrong".to_string(),
                    type_node: slice_i64.clone(),
                }],
                return_type: MirType::Void,
                locals: vec![],
                blocks: vec![MirBlock {
                    label: "bb0".to_string(),
                    instructions: vec![MirInstruction::Call {
                        target: Some(MirValue::Temp {
                            name: "result".to_string(),
                            type_node: slice_i64.clone(),
                        }),
                        function_name: "identity".to_string(),
                        args: vec![MirValue::Param {
                            name: "wrong".to_string(),
                            type_node: slice_i64.clone(),
                        }],
                    }],
                    terminator: MirTerminator::Return { value: None },
                }],
            },
            MirFunction {
                name: "bad_return".to_string(),
                exported: false,
                params: vec![MirParam {
                    name: "wrong".to_string(),
                    type_node: slice_i64.clone(),
                }],
                return_type: slice_i32,
                locals: vec![],
                blocks: vec![MirBlock {
                    label: "bb0".to_string(),
                    instructions: vec![],
                    terminator: MirTerminator::Return {
                        value: Some(MirValue::Param {
                            name: "wrong".to_string(),
                            type_node: slice_i64,
                        }),
                    },
                }],
            },
        ],
    };

    let messages = validate_mir_module(&module)
        .errors
        .into_iter()
        .map(|error| error.message)
        .collect::<Vec<_>>();
    assert!(
        messages
            .iter()
            .any(|message| message.contains("void slice element")),
        "{messages:#?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("direct slice element")),
        "{messages:#?}"
    );
    assert!(
        messages
            .iter()
            .any(|message| message.contains("exported slice return")),
        "{messages:#?}"
    );
    for mismatch in ["Call argument", "Call result", "Return type mismatch"] {
        assert!(
            messages.iter().any(|message| message.contains(mismatch)),
            "missing {mismatch}: {messages:#?}"
        );
    }
}

#[test]
fn mir_cli_should_match_typescript_oracle_for_official_examples_across_opt_levels() {
    let Some(ts_cli) = typescript_cli() else {
        return;
    };
    let examples = fixtures::ORACLE_EXAMPLES
        .iter()
        .chain(fixtures::BENCHMARK_FIXTURES)
        .map(|fixture| fixture.oracle)
        .chain(std::iter::once("tests/fixtures/f64_edges.ck"))
        .collect::<Vec<_>>();

    for opt_level in 0..=3 {
        let opt_flag = format!("-O{opt_level}");
        for &example in &examples {
            let source = typescript_root().join(example);

            let ts_output = Command::new("node")
                .arg(&ts_cli)
                .arg("emit-mir")
                .arg(&opt_flag)
                .arg(&source)
                .output()
                .expect("run TypeScript emit-mir");
            assert!(
                ts_output.status.success(),
                "{example} {opt_flag} TS stderr:\n{}",
                String::from_utf8_lossy(&ts_output.stderr)
            );

            let rust_output = Command::new(env!("CARGO_BIN_EXE_ckc"))
                .arg("emit-mir")
                .arg(&opt_flag)
                .arg(&source)
                .output()
                .expect("run Rust emit-mir");
            assert!(
                rust_output.status.success(),
                "{example} {opt_flag} Rust stderr:\n{}",
                String::from_utf8_lossy(&rust_output.stderr)
            );

            assert_eq!(
                String::from_utf8(rust_output.stdout).expect("Rust MIR should be UTF-8"),
                String::from_utf8(ts_output.stdout).expect("TS MIR should be UTF-8"),
                "{example} {opt_flag}"
            );
            assert_eq!(
                String::from_utf8(rust_output.stderr).expect("Rust stderr should be UTF-8"),
                String::from_utf8(ts_output.stderr).expect("TS stderr should be UTF-8"),
                "{example} {opt_flag} stderr"
            );
        }
    }
}
