use std::{fs, process::Command};

use calckernel::{
    BoundsMode, KirConsumer, KirInstructionKind, KirTerminator, MirPrimitiveTypeName,
    MirRuntimeIntrinsic, MirType, OverflowMode, emit_c_kir_module_with_contracts,
};

use super::support::{
    compiler::{optimized_module, verified_artifact},
    temp::unique_id,
};

#[test]
fn optimizer_should_preserve_break_continue_cfg_at_all_opt_levels() {
    let source = r#"
      export fn control(n: u32) -> u32 {
        let i: u32 = 0;
        let sum: u32 = 0;
        while i < n {
          i = i + 1;
          if i == 2 { continue; }
          sum = sum + i;
          if sum > 10 { break; }
        }
        return sum;
      }
    "#;
    for level in 0..=3 {
        let result = optimized_module(
            source,
            level,
            KirConsumer::Inspection,
            OverflowMode::Unchecked,
            BoundsMode::Unchecked,
        );
        let function = verified_artifact(&result)
            .functions
            .iter()
            .find(|function| function.name == "control")
            .expect("control function");
        assert!(function.exported, "O{level}");
        assert!(
            function.blocks.iter().any(|block| matches!(
                block.terminator,
                KirTerminator::Return { value: Some(_), .. }
            )),
            "O{level}"
        );
        assert!(
            result.records.iter().all(|record| record.verified),
            "O{level}"
        );
    }
}

#[test]
fn optimizer_should_preserve_runtime_print_count_and_order_at_all_levels() {
    let source = "fn main() -> void { print_i32(1); print_bool(true); print_newline(); }";
    for level in 0..=3 {
        let result = optimized_module(
            source,
            level,
            KirConsumer::Inspection,
            OverflowMode::Unchecked,
            BoundsMode::Unchecked,
        );
        let main = verified_artifact(&result)
            .functions
            .iter()
            .find(|function| function.name == "main")
            .expect("main function");
        let calls = main
            .blocks
            .iter()
            .flat_map(|block| &block.instructions)
            .filter_map(|instruction| match instruction.kind {
                KirInstructionKind::RuntimeCall { intrinsic, .. } => Some((
                    intrinsic,
                    instruction.effect.as_ref().expect("ordered effect").order,
                )),
                _ => None,
            })
            .collect::<Vec<_>>();
        assert_eq!(
            calls,
            vec![
                (MirRuntimeIntrinsic::PrintI32, 0),
                (MirRuntimeIntrinsic::PrintBool, 1),
                (MirRuntimeIntrinsic::PrintNewline, 2),
            ],
            "O{level}"
        );
    }
}

#[test]
fn optimizer_should_preserve_slice_internal_calls_and_returns() {
    let source = r#"
      fn identity(items: slice<i32>) -> slice<i32> { return items; }
      fn forward(items: slice<i32>) -> slice<i32> { return identity(items); }
      export fn returned_data(data: ptr<i32>, len: u32) -> ptr<i32> {
        let items: slice<i32> = forward(slice(data, len));
        return items.data;
      }
      export fn returned_len(data: ptr<i32>, len: u32) -> u32 {
        let items: slice<i32> = forward(slice(data, len));
        return items.len;
      }
    "#;
    let expected = MirType::Slice(Box::new(MirType::Primitive(MirPrimitiveTypeName::I32)));
    for level in 0..=3 {
        let result = optimized_module(
            source,
            level,
            KirConsumer::C,
            OverflowMode::Unchecked,
            BoundsMode::Checked,
        );
        let module = verified_artifact(&result);
        for name in ["identity", "forward"] {
            let function = module
                .functions
                .iter()
                .find(|function| function.name == name);
            if level <= 1 {
                assert!(function.is_some(), "O{level}: non-inlined {name}");
            }
            if let Some(function) = function {
                assert_eq!(function.return_type, expected, "O{level}: {name}");
                assert_eq!(function.params[0].type_node, expected, "O{level}: {name}");
            }
        }
        // O2/O3 may inline the calls. Execute the data/length identity instead of
        // requiring a legacy MIR call spelling after a legal transformation.
        let c = emit_c_kir_module_with_contracts(module, result.contract_facts.as_ref())
            .expect("checked slice C lowering");
        let harness = format!(
            r#"
{c}
int main(void) {{
    int32_t values[4] = {{10, 20, 30, 40}};
    const uint32_t lengths[3] = {{0, 1, 4}};
    for (uint32_t i = 0; i < 3; ++i) {{
        int32_t* actual_data = (int32_t*)0;
        uint32_t actual_len = 99;
        if (returned_data(values, lengths[i], &actual_data) != CK_OK) return 1;
        if (returned_len(values, lengths[i], &actual_len) != CK_OK) return 2;
        if (actual_data != values || actual_len != lengths[i]) return 3;
    }}
    int32_t* empty_data = values;
    uint32_t empty_len = 99;
    if (returned_data((int32_t*)0, 0, &empty_data) != CK_OK) return 4;
    if (returned_len((int32_t*)0, 0, &empty_len) != CK_OK) return 5;
    return empty_data == (int32_t*)0 && empty_len == 0 ? 0 : 6;
}}
"#
        );
        let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("target")
            .join(format!("slice-preservation-{}", unique_id()));
        fs::create_dir_all(&directory).expect("create owned fixture directory");
        let input = directory.join("roundtrip.c");
        let executable = directory.join("roundtrip");
        fs::write(&input, harness).expect("write generated C fixture");
        let compilation = Command::new("clang")
            .args(["-std=c11", "-Wall", "-Wextra", "-Werror"])
            .arg(&input)
            .arg("-o")
            .arg(&executable)
            .output()
            .expect("compile slice round-trip fixture");
        assert!(compilation.status.success(), "O{level}: {compilation:?}");
        let output = Command::new(&executable)
            .output()
            .expect("execute slice round trip");
        assert_eq!(output.status.code(), Some(0), "O{level}: {output:?}");
        assert!(
            output.stdout.is_empty() && output.stderr.is_empty(),
            "O{level}: {output:?}"
        );
        fs::remove_dir_all(directory).expect("remove owned generated fixture");
    }
}
