use calckernel::{
    BoundsMode, CkLateProfileFunctionLayout, CkLateProfileLayoutPlan, CkLateProfileRepair,
    EmitLlvmOptions, KirConsumer, NativeContext, NativeOptimizationLevel, NativeTarget,
    OverflowMode, lower_native_kir_module, test_apply_late_layout_bytes,
};

use super::support::compiler::optimized_module;

const SOURCE: &str = "
export fn choose(items: slice<i32>, value: i32) -> i32 {
  if value > 0 { items[0] = value; return items[1]; }
  if value < 0 { items[1] = value; return items[0]; }
  return items[2];
}";

fn optimized<'context>(
    context: &'context NativeContext,
    target: &NativeTarget,
) -> calckernel::OptimizedNativeModule<'context> {
    let kir = optimized_module(
        SOURCE,
        2,
        KirConsumer::NativeLibrary,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    lower_native_kir_module(context, target, &kir, &EmitLlvmOptions::default())
        .expect("lower O2 layout fixture")
        .verify()
        .expect("verify O2 layout fixture")
        .audit()
        .expect("audit O2 layout fixture")
        .optimize(target, NativeOptimizationLevel::O2)
        .expect("optimize O2 layout fixture")
}

#[test]
fn pgo_layout_o2_boundary_should_be_profile_blind_and_accept_only_block_permutation() {
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    let ordinary = optimized(&context, &target);
    let ordinary_ir = ordinary.to_ir_string().expect("ordinary O2 IR");
    for forbidden in [
        "!prof",
        "function_entry_count",
        "branch_weights",
        "\"hot\"",
        "\"cold\"",
    ] {
        assert!(
            !ordinary_ir.contains(forbidden),
            "{forbidden}:\n{ordinary_ir}"
        );
    }
    let ordinary_snapshot = ordinary
        .late_layout_snapshot(&target)
        .expect("ordinary frozen snapshot");
    assert!(!ordinary_snapshot.accepted);
    assert_eq!(ordinary_snapshot.reason, "no-layout-authority");

    let profiled = optimized(&context, &target);
    let profiled_snapshot = profiled
        .late_layout_snapshot(&target)
        .expect("profile frozen snapshot");
    assert_eq!(
        ordinary_snapshot.pre_layout_digest, profiled_snapshot.pre_layout_digest,
        "profile analysis must not reach the pre-layout pipeline"
    );
    let mut blocks = llvm_kir_blocks(&ordinary_ir);
    assert!(
        blocks.len() >= 2,
        "fixture must retain real O2 blocks:\n{ordinary_ir}"
    );
    blocks.reverse();
    let plan = CkLateProfileLayoutPlan {
        functions: vec![CkLateProfileFunctionLayout {
            llvm_function: "choose".to_string(),
            blocks,
        }],
    };
    let (profiled, report) = profiled
        .apply_late_profile_layout(&target, &plan)
        .expect("apply real late layout");
    assert!(report.accepted, "{}", report.reason);
    assert!(report.changed, "fixture plan must change order");
    assert_ne!(report.pre_layout_digest, report.post_layout_digest);
    assert_eq!(report.pre_structural_digest, report.post_structural_digest);
    assert!(
        report
            .repairs
            .contains(&CkLateProfileRepair::FallthroughTerminator)
    );
    assert!(
        report
            .repairs
            .contains(&CkLateProfileRepair::AlignmentPadding)
    );
    if cfg!(target_arch = "aarch64") {
        assert!(
            report
                .repairs
                .contains(&CkLateProfileRepair::BranchRelaxation)
        );
    } else {
        assert!(report.repairs.contains(&CkLateProfileRepair::BranchFixup));
    }
    let object = target.emit_object(profiled).expect("emit reordered object");
    assert!(!object.is_empty());
}

#[test]
fn pgo_layout_bridge_should_reject_malformed_or_forged_records_without_object() {
    let context = NativeContext::new().expect("native context");
    let target = NativeTarget::host().expect("native target");
    let module = optimized(&context, &target);
    let malformed = test_apply_late_layout_bytes(&module, &target, b"CKLAYOUT0\n")
        .expect_err("reject schema mutation");
    assert!(malformed.message.contains("invalid schema"));
    let forged =
        test_apply_late_layout_bytes(&module, &target, b"CKLAYOUT1\nB\tchoose\tkir.bb999999\n")
            .expect_err("reject unknown block");
    assert!(forged.message.contains("unknown block"));
}

fn llvm_kir_blocks(ir: &str) -> Vec<String> {
    let mut inside = false;
    let mut blocks = Vec::new();
    for line in ir.lines() {
        if line.starts_with("define ") && line.contains("@choose(") {
            inside = true;
            continue;
        }
        if inside && line == "}" {
            break;
        }
        if inside
            && let Some((label, _)) = line.trim().split_once(':')
            && label != "entry"
            && !label.is_empty()
        {
            blocks.push(label.to_string());
        }
    }
    blocks
}
