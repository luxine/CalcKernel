use calckernel::{
    BoundsMode, EmitWasmOptions, KirConsumer, OverflowMode, emit_wasm_kir_module, print_kir_module,
};

use crate::support::compiler::{optimized_module, verified_artifact};

#[test]
fn modular_affine_should_compose_two_fixed_steps_at_o3() {
    let optimized = optimized_module(
        "export fn compose(x: u32) -> u32 { let y: u32 = x * 1664525 + 1013904223; return y * 1664525 + 1013904223; }",
        3,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let text = print_kir_module(verified_artifact(&optimized));
    assert!(text.contains("const_int 389569705"), "{text}");
    assert!(text.contains("const_int 1196435762"), "{text}");
}

#[test]
fn modular_affine_should_preserve_checked_and_strict_float_operations() {
    for (source, overflow) in [
        (
            "export fn compose(x: u32) -> u32 { let y: u32 = x * 7 + 11; return y * 13 + 17; }",
            OverflowMode::Checked,
        ),
        (
            "export fn compose(x: f64) -> f64 { let y: f64 = x * 7.0 + 11.0; return y * 13.0 + 17.0; }",
            OverflowMode::Unchecked,
        ),
        (
            "export fn compose(x: u64) -> u64 { let y: u64 = x * 7 + 11; return y * 13 + 17; }",
            OverflowMode::Unchecked,
        ),
    ] {
        let optimized = optimized_module(
            source,
            3,
            KirConsumer::Inspection,
            overflow,
            BoundsMode::Unchecked,
        );
        assert!(
            optimized
                .records
                .iter()
                .any(|record| record.name == "modular-affine-composition" && !record.changed)
        );
    }
}

#[test]
fn modular_affine_should_not_duplicate_an_observed_outer_product() {
    for (body, expected) in [
        ("out[0] = product; return product + 17;", false),
        (
            "let result: u32 = product + 17; if flag { return product; } return result;",
            false,
        ),
        ("out[0] = y; return product + 17;", true),
    ] {
        let source = format!(
            "export fn compose(out: ptr<u32>, x: u32, flag: bool) -> u32 {{ let y: u32 = x * 7 + 11; let product: u32 = y * 13; {body} }}"
        );
        let optimized = optimized_module(
            &source,
            3,
            KirConsumer::WebAssembly,
            OverflowMode::Unchecked,
            BoundsMode::Unchecked,
        );
        assert_eq!(
            optimized
                .records
                .iter()
                .find(|record| record.name == "modular-affine-composition")
                .expect("pass record")
                .changed,
            expected,
            "{source}"
        );
    }
}

#[test]
fn modular_affine_should_preserve_wrapping_results_and_observed_intermediates() {
    if !crate::support::command::node_available() {
        eprintln!("skipping modular affine runtime check: node unavailable");
        return;
    }
    let source = r#"
        export fn unsigned(x: u32) -> u32 {
            let y: u32 = x * 4294967295 + 1013904223;
            return y * 1664525 + 1013904223;
        }
        export fn signed(x: i32) -> i32 {
            let y: i32 = x * 1664525 + 1013904223;
            return y * 2000 + 1013904223;
        }
        export fn observed(out: ptr<u32>, x: u32) -> u32 {
            let y: u32 = x * 7 + 11;
            out[0] = y;
            let product: u32 = y * 13;
            out[1] = product;
            return product + 17;
        }
    "#;
    let bytes = [0, 3].map(|opt_level| {
        let optimized = optimized_module(
            source,
            opt_level,
            KirConsumer::WebAssembly,
            OverflowMode::Unchecked,
            BoundsMode::Unchecked,
        );
        emit_wasm_kir_module(verified_artifact(&optimized), EmitWasmOptions { opt_level })
            .expect("WASM")
    });
    let script = r#"
        const assert = require('node:assert/strict');
        const modules = JSON.parse(process.argv[1]).map(bytes => new WebAssembly.Instance(new WebAssembly.Module(Uint8Array.from(bytes))).exports);
        const seeds = [0, 1, 0x7fffffff, 0x80000000, 0xfffffffe, 0xffffffff];
        let state = 123456789;
        for (let i = 0; i < 4096; i++) { state = (Math.imul(state, 1103515245) + 12345) >>> 0; seeds.push(state); }
        for (const x of seeds) {
            for (const wasm of modules) {
                const unsignedY = (Math.imul(x, 0xffffffff) + 1013904223) >>> 0;
                const signedY = (Math.imul(x, 1664525) + 1013904223) | 0;
                assert.equal(wasm.unsigned(x) >>> 0, (Math.imul(unsignedY, 1664525) + 1013904223) >>> 0);
                assert.equal(wasm.signed(x), (Math.imul(signedY, 2000) + 1013904223) | 0);
                const y = (Math.imul(x, 7) + 11) >>> 0;
                const product = Math.imul(y, 13) >>> 0;
                assert.equal(wasm.observed(1024, x) >>> 0, (product + 17) >>> 0);
                assert.deepEqual([...new Uint32Array(wasm.memory.buffer, 1024, 2)], [y, product]);
            }
        }
    "#;
    let output = std::process::Command::new("node")
        .args([
            "-e",
            script,
            &serde_json::to_string(&bytes).expect("module JSON"),
        ])
        .output()
        .expect("node");
    assert!(
        output.status.success(),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}
