use super::support::compiler::{optimized_module, verified_artifact};
use calckernel::{
    BoundsMode, EmitWasmOptions, KirConsumer, OverflowMode, emit_wasm_kir_module,
    emit_wat_kir_module,
};
use std::{fs, process::Command};

const SOURCE: &str = r#"
export unsafe fn row(input: slice<u32>, out: slice<u32>, offset: u32, n: u32) -> void contract {
  requires n <= out.len;
  effects read(input), readwrite(out);
} {
  let i: u32 = 0;
  while i < n {
    let value: u32 = input[offset + i];
    if value != 0 { out[i] = value; }
    i = i + 1;
  }
}
export unsafe fn row_f64(input: slice<f64>, out: slice<f64>, offset: u32, n: u32) -> void contract {
  requires n <= out.len;
  effects read(input), readwrite(out);
} {
  let i: u32 = 0;
  while i < n {
    let value: f64 = input[offset + i];
    if value != 1.0 { out[i] = value; }
    i = i + 1;
  }
}
"#;

#[test]
fn multiblock_modular_row_address_is_factored_from_source_invariance() {
    let optimized = optimized_module(
        SOURCE,
        3,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let wat = emit_wat_kir_module(
        verified_artifact(&optimized),
        EmitWasmOptions { opt_level: 3 },
    )
    .unwrap();
    assert!(
        wat.contains("ik_factored_base"),
        "multi-block modular row addresses use a preheader base"
    );
    wat::parse_str(&wat).unwrap();
}

#[test]
fn factored_addresses_preserve_u32_byte_wrap_and_partial_write_traps() {
    if !super::support::command::node_available() {
        return;
    }
    let optimized = optimized_module(
        SOURCE,
        3,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let module = verified_artifact(&optimized);
    let before = emit_wasm_kir_module(module, EmitWasmOptions { opt_level: 0 }).unwrap();
    let after = emit_wasm_kir_module(module, EmitWasmOptions { opt_level: 3 }).unwrap();
    let dir = super::support::temp::temp_dir("ck-wasm-factored-row");
    fs::create_dir_all(&dir).unwrap();
    fs::write(dir.join("before.wasm"), before).unwrap();
    fs::write(dir.join("after.wasm"), after).unwrap();
    fs::write(dir.join("run.cjs"), r#"
const fs=require('node:fs'),assert=require('node:assert/strict');
(async()=>{
 const engines=[];for(const p of process.argv.slice(2))engines.push((await WebAssembly.instantiate(fs.readFileSync(p))).instance.exports);
 let cases=0;
 for(const n of [0,1,2,3,4,5,7,8,9,16,17])for(const [base,offset] of [[4096,0],[4096,0x40000000],[4096,0xffffffff],[65532,0],[65528,0],[4,0xffffffff]])for(const output of [2048,4096,4104])for(const operation of ['row','row_f64']){
  let expected;
  for(const w of engines){
   const a=new Uint32Array(w.memory.buffer);for(let i=0;i<a.length;i++)a[i]=i%3?i:0;
   // Exact source bits include +0, -0, payload NaNs, infinities, and 1.0.
   a.set([0,0,0,0x80000000,0x1234,0x7ff80000,1,0x7ff00000,0,0x7ff00000,0,0xfff00000,0,0x3ff00000],1024);
   let status='ok';try{w[operation](base,n,output,n,offset,n);}catch(e){assert(e instanceof WebAssembly.RuntimeError);status='trap';}
   const result=[status,Buffer.from(w.memory.buffer).toString('hex')];if(expected)assert.deepEqual(result,expected,`${operation} n=${n} base=${base} offset=${offset} out=${output}`);else expected=result;
  }cases++;
 }process.stdout.write(String(cases));
})().catch(e=>{console.error(e);process.exitCode=1});
"#).unwrap();
    let result = Command::new("node")
        .arg(dir.join("run.cjs"))
        .arg(dir.join("before.wasm"))
        .arg(dir.join("after.wasm"))
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(dir);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "396");
}

const SIMD_MATMUL: &str = r#"
export unsafe fn matmul_column(a: slice<f64>, b: slice<f64>, out: slice<f64>, n: u32) -> void
contract {
  requires n != 0 && n <= a.len && n <= b.len && n <= out.len;
  requires noalias(a, b) && noalias(a, out) && noalias(b, out);
  effects read(a), read(b), readwrite(out);
} {
  let row: u32 = 0;
  while row < n {
    let inner: u32 = 0;
    while inner < n {
      let col: u32 = 0;
      while col < n {
        let oi: u32 = row * n + col;
        let ai: u32 = row * n + inner;
        let bi: u32 = inner * n + col;
        out[oi] = out[oi] + a[ai] * b[bi];
        col = col + 1;
      }
      inner = inner + 1;
    }
    row = row + 1;
  }
}
"#;

fn simd_matmul_module() -> calckernel::KirModule {
    use calckernel::*;
    let checked = check(&SourceFile::new("factored-simd.ck", SIMD_MATMUL));
    assert!(checked.diagnostics.is_empty());
    let module = build_kir_module_with_profile(
        &lower_to_mir(&checked.checked_program).unwrap(),
        KirBuildConfig {
            consumer: KirConsumer::WebAssembly,
            overflow_mode: KirOverflowMode::Unchecked,
            bounds_mode: KirBoundsMode::Unchecked,
            sanitizer_mode: KirSanitizerMode::Disabled,
        },
        KirTargetProfile::webassembly_with_features(KirWasmFeatures::Simd128),
    )
    .unwrap();
    // Import contracts after scalar normalization so this test targets the
    // affine UF4 backend independently of optional outer-loop versioning.
    let optimized = run_kir_pass_pipeline(module, KirOptimizationLevel::O3, None);
    assert!(optimized.errors.is_empty(), "{:?}", optimized.errors);
    let module = optimized.artifact.unwrap();
    let contracts = import_contract_facts(&module, &checked.checked_program, 0).unwrap();
    let state = KirVerifiedProgramState::from_parts(
        module,
        Some(contracts),
        optimized.proofs,
        optimized.eliminated_guards,
        0,
    )
    .unwrap();
    let candidate = discover_vectorization_candidates(&state)
        .candidates
        .into_iter()
        .find(|candidate| candidate.vf == 2 && candidate.uf == 4 && candidate.wasm_affine.is_some())
        .expect("UF4 affine candidate");
    let prepared = prepare_vectorization_trial(&state, &candidate).unwrap();
    check_vectorization_trial_independently(
        &state,
        &prepared.trial,
        &prepared.plan,
        &prepared.charge,
    )
    .unwrap();
    prepared.trial.module().clone()
}

#[test]
fn simd_affine_bundle_reuses_modular_byte_addresses_in_typed_emission() {
    let module = simd_matmul_module();
    let wat = emit_wat_kir_module(&module, EmitWasmOptions { opt_level: 3 }).unwrap();
    assert!(
        wat.contains("ik_bundle_base"),
        "UF4 affine vector addresses should share byte bases"
    );
    let direct = emit_wasm_kir_module(&module, EmitWasmOptions { opt_level: 3 }).unwrap();
    let text = wat::parse_str(&wat).unwrap();
    wasmparser::Validator::new().validate_all(&direct).unwrap();
    wasmparser::Validator::new().validate_all(&text).unwrap();
}

#[test]
fn simd_factored_bundle_preserves_trap_prefixes_and_non_nan_bytes() {
    if !super::support::command::node_available() {
        return;
    }
    let module = simd_matmul_module();
    let before = emit_wasm_kir_module(&module, EmitWasmOptions { opt_level: 0 }).unwrap();
    let after = emit_wasm_kir_module(&module, EmitWasmOptions { opt_level: 3 }).unwrap();
    let wat = emit_wat_kir_module(&module, EmitWasmOptions { opt_level: 3 }).unwrap();
    assert!(wat.contains("ik_bundle_base"));
    let text = wat::parse_str(&wat).unwrap();
    let dir = super::support::temp::temp_dir("ck-wasm-simd-bundle");
    fs::create_dir_all(&dir).unwrap();
    for (name, bytes) in [("before", before), ("after", after), ("text", text)] {
        fs::write(dir.join(format!("{name}.wasm")), bytes).unwrap();
    }
    fs::write(dir.join("check.cjs"), r#"
const fs=require('node:fs'), assert=require('node:assert/strict');
const mods=process.argv.slice(2).map(p=>new WebAssembly.Module(fs.readFileSync(p)));
const finiteBits=[0n,0x8000000000000000n,0x3fe0000000000000n,0x3ff0000000000000n,0xbff0000000000000n,0x4000000000000000n,0x400a000000000000n,0xc010000000000000n];
const nanBits=[0n,0x8000000000000000n,1n,0x3ff0000000000000n,0xbff0000000000000n,0x7ff8000000000042n,0x7ff8000012345678n,0x7ff0000000000001n,0x7ff0000000000000n,0xfff0000000000000n];
const cases=[];
for(const n of [0,1,2,7,8,15,16,17,23,24,31,32,33,63,64,65])for(const limited of [false,true]){
 const len=limited?Math.max(n,1):Math.max(n*n,1),a=4096,b=a+len*8+64,out=b+len*8+64;
 cases.push({n,len,a,b,out});
}
for(const key of ['a','b','out'])for(const remain of [1,7,8,15,16]){const c={n:32,len:1024,a:4096,b:16384,out:32768};c[key]=131072-remain*8;cases.push(c);}
cases.push({n:17,len:289,a:0xfffffff8,b:16384,out:32768});
let checks=0;
const corpora=[
 {name:'finite',bits:finiteBits,compareNaNs:false},
 {name:'nan',bits:nanBits,compareNaNs:true}
];
for(const corpus of corpora)for(const c of cases){let expected;
 for(const mod of mods){const w=new WebAssembly.Instance(mod).exports;if(w.memory.buffer.byteLength<131072)w.memory.grow(1);
 const raw=new Uint8Array(w.memory.buffer),view=new DataView(raw.buffer);
 for(let i=0;i<raw.length;i+=8)view.setBigUint64(i,0x405edd2f1a9fbe77n,true);
 for(const [ptr,seed] of [[c.a,1],[c.b,3]])for(let i=0;i<c.len&&ptr+i*8+8<=raw.length;i++)view.setBigUint64(ptr+i*8,corpus.bits[(i+seed)%corpus.bits.length],true);
 for(let i=0;i<c.n*c.n&&c.out+i*8+8<=raw.length;i++)view.setBigUint64(c.out+i*8,0x4059000000000000n,true);
 const initial=Buffer.from(raw);let trapped=false;try{w.matmul_column(c.a,c.len,c.b,c.len,c.out,c.len,c.n);}catch(e){assert(e instanceof WebAssembly.RuntimeError);trapped=true;}
 const result={trapped,bytes:Buffer.from(raw),initial};
 if(expected){
  assert.equal(result.trapped,expected.trapped,`${corpus.name} trap status ${JSON.stringify(c)}`);
  if(corpus.compareNaNs){
   const initialView=new DataView(expected.initial.buffer,expected.initial.byteOffset,expected.initial.byteLength);
   const expectedView=new DataView(expected.bytes.buffer,expected.bytes.byteOffset,expected.bytes.byteLength);
   const actualView=new DataView(result.bytes.buffer,result.bytes.byteOffset,result.bytes.byteLength);
   const outputStarts=new Set();
   for(let row=0;row<c.n;row++)for(let col=0;col<c.n;col++){
    const index=(row*c.n+col)>>>0,offset=(c.out+Math.imul(index,8))>>>0;
    if(offset+8<=result.bytes.length)outputStarts.add(offset);
   }
   for(let offset=0;offset<result.bytes.length;offset+=8){
    const same=result.bytes.subarray(offset,offset+8).equals(expected.bytes.subarray(offset,offset+8));
    if(same)continue;
    const expectedBits=expectedView.getBigUint64(offset,true),actualBits=actualView.getBigUint64(offset,true);
    const arithmeticNaN=outputStarts.has(offset)
      && !Number.isNaN(initialView.getFloat64(offset,true))
      && Number.isNaN(expectedView.getFloat64(offset,true))
      && Number.isNaN(actualView.getFloat64(offset,true))
      && (expectedBits&0x0008000000000000n)!==0n
      && (actualBits&0x0008000000000000n)!==0n;
    assert(arithmeticNaN,`non-NaN memory or write-prefix differs at byte ${offset}: ${JSON.stringify(c)}`);
   }
  }else assert(result.bytes.equals(expected.bytes),`finite memory/write-prefix differs: ${JSON.stringify(c)}`);
  checks++;
 }else expected=result;
 }
}
let negativeZeroExpected;
for(const mod of mods){
 const w=new WebAssembly.Instance(mod).exports;if(w.memory.buffer.byteLength<131072)w.memory.grow(1);
 const raw=new Uint8Array(w.memory.buffer),view=new DataView(raw.buffer);
 for(let i=0;i<raw.length;i+=8)view.setBigUint64(i,0x405edd2f1a9fbe77n,true);
 const a=4096,b=4160,out=4224,negativeZero=0x8000000000000000n;
 view.setBigUint64(a,negativeZero,true);
 view.setBigUint64(b,0x3ff0000000000000n,true);
 view.setBigUint64(out,negativeZero,true);
 let trapped=false;try{w.matmul_column(a,1,b,1,out,1,1);}catch(e){assert(e instanceof WebAssembly.RuntimeError);trapped=true;}
 assert.equal(trapped,false,'signed-zero case must not trap');
 assert.equal(view.getBigUint64(out,true),negativeZero,'finite arithmetic must preserve the exact negative-zero output bits');
 const bytes=Buffer.from(raw);
 if(negativeZeroExpected){assert(bytes.equals(negativeZeroExpected), 'signed-zero memory differs');checks++;}
 else negativeZeroExpected=bytes;
}
console.log(checks);
"#).unwrap();
    let result = Command::new("node")
        .arg(dir.join("check.cjs"))
        .args([
            dir.join("before.wasm"),
            dir.join("after.wasm"),
            dir.join("text.wasm"),
        ])
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(dir);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap().trim(), "194");
}
