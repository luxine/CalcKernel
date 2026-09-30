use super::support::compiler::{optimized_module, verified_artifact};
use calckernel::{
    BoundsMode, CandidateDisposition, CandidateKey, EmitWasmOptions, KirConsumer,
    LoopCandidateKind, OverflowMode, emit_wasm_kir_module, emit_wat_kir_module,
};
use std::{fs, process::Command};

const SOURCE: &str = r#"
export unsafe fn sum(input: slice<u32>, out: slice<u32>) -> void contract {
  requires out.len == 1;
  requires noalias(input, out);
  effects read(input), write(out);
} {
  let i: u32 = 0;
  let total: u32 = 0;
  while i < input.len {
    total = total + input[i];
    i = i + 1;
  }
  out[0] = total;
}

export unsafe fn transform(input: slice<f64>, out: slice<f64>) -> void contract {
  requires input.len == out.len;
  requires noalias(input, out);
  effects read(input), write(out);
} {
  let i: u32 = 0;
  while i < input.len {
    out[i] = input[i] * 0.5 + 0.25;
    i = i + 1;
  }
}
"#;

fn emit_pair(source: &str) -> (String, Vec<u8>, String, Vec<u8>) {
    let baseline = optimized_module(
        source,
        0,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let optimized = optimized_module(
        source,
        3,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    );
    let baseline_module = verified_artifact(&baseline);
    let optimized_module = verified_artifact(&optimized);
    let baseline_wat = emit_wat_kir_module(baseline_module, EmitWasmOptions { opt_level: 0 })
        .expect("baseline WAT");
    let optimized_wat = emit_wat_kir_module(optimized_module, EmitWasmOptions { opt_level: 3 })
        .expect("optimized WAT");
    let baseline_wasm = emit_wasm_kir_module(baseline_module, EmitWasmOptions { opt_level: 0 })
        .expect("baseline direct binary");
    let optimized_wasm = emit_wasm_kir_module(optimized_module, EmitWasmOptions { opt_level: 3 })
        .expect("optimized direct binary");
    let accepted = optimized
        .audit
        .attempts()
        .iter()
        .filter(|attempt| {
            matches!(
                attempt.key,
                CandidateKey::LoopFrontier {
                    kind: LoopCandidateKind::RuntimeScalarUnroll,
                    ..
                }
            ) && attempt.disposition == CandidateDisposition::Accepted
        })
        .count();
    assert_eq!(
        accepted,
        2,
        "both frozen shapes must take the checked O3 transaction:\n{}",
        calckernel::print_optimization_audit(&optimized.audit)
    );
    (baseline_wat, baseline_wasm, optimized_wat, optimized_wasm)
}

#[test]
fn baseline_runtime_unroll_is_accepted_and_emits_a_guarded_four_lane_path() {
    let (baseline_wat, _, optimized_wat, _) = emit_pair(SOURCE);
    assert!(!baseline_wat.contains("i32.rem_u"));
    assert!(
        optimized_wat.contains("i32.rem_u"),
        "UF4 limit must use the bound remainder:\n{optimized_wat}"
    );
    assert!(
        optimized_wat.contains("i32.ge_u"),
        "fast path must retain its nontrapping minimum-trip guard:\n{optimized_wat}"
    );
    wat::parse_str(&baseline_wat).expect("baseline WAT parses");
    wat::parse_str(&optimized_wat).expect("optimized WAT parses");
}

#[test]
fn baseline_runtime_unroll_preserves_u32_wrap_strict_float_bits_and_trap_prefix() {
    if !super::support::command::node_available() {
        return;
    }
    let (baseline_wat, baseline_wasm, optimized_wat, optimized_wasm) = emit_pair(SOURCE);
    let baseline_wat_wasm = wat::parse_str(&baseline_wat).expect("baseline WAT binary");
    let optimized_wat_wasm = wat::parse_str(&optimized_wat).expect("optimized WAT binary");
    let dir = super::support::temp::temp_dir("ck-wasm-runtime-scalar-uf4");
    fs::create_dir_all(&dir).expect("test temp directory");
    let paths = [
        ("baseline-direct.wasm", baseline_wasm),
        ("optimized-direct.wasm", optimized_wasm),
        ("baseline-wat.wasm", baseline_wat_wasm),
        ("optimized-wat.wasm", optimized_wat_wasm),
    ];
    for (name, bytes) in paths {
        fs::write(dir.join(name), bytes).expect("write Wasm module");
    }
    fs::write(
        dir.join("run.cjs"),
        r#"
const fs=require('node:fs'),assert=require('node:assert/strict');
(async()=>{
 const engines=[];
 for(const path of process.argv.slice(2))engines.push((await WebAssembly.instantiate(fs.readFileSync(path))).instance.exports);
 const lengths=Array.from({length:18},(_,i)=>i);
 const floatBits=[
  0x0000000000000000n,0x8000000000000000n,0x3ff0000000000000n,
  0xbff0000000000000n,0x7ff8000000001234n,0x7ff0000000000000n,
  0xfff0000000000000n,0x0000000000000001n,0x0010000000000000n,
  0x400921fb54442d18n,0x3fd5555555555555n,0x7fefffffffffffffn,
  0xfff8000000005678n,0x3fe0000000000000n,0xc004000000000000n,
  0x0008000000000000n,0x4014000000000000n,0x3ca0000000000000n
 ];
 function runSum(w,n,physicalTrap=false){
  const bytes=new Uint8Array(w.memory.buffer);bytes.fill(0xa5);
  const input=physicalTrap?65520:4096,out=16384;
  const words=new Uint32Array(w.memory.buffer);
  for(let i=0;i<n;i++)words[input/4+i]=i%3===0?0xffffffff:(i*0x10203041+7)>>>0;
  let status='ok';try{w.sum(input,n,out,1)}catch(e){assert(e instanceof WebAssembly.RuntimeError);status='trap'}
  if(status==='ok'){
   let expected=0;for(let i=0;i<n;i++)expected=(expected+words[input/4+i])>>>0;
   assert.equal(words[out/4],expected,`u32 modular oracle n=${n}`);
  }else if(physicalTrap){
   assert.equal(words[out/4],0xa5a5a5a5,`sum must not commit before a trapping load n=${n}`);
  }
  return [status,Buffer.from(w.memory.buffer).toString('hex')];
 }
 function runMap(w,n,physicalTrap=false){
  const bytes=new Uint8Array(w.memory.buffer);bytes.fill(0xa5);
  const input=4096,out=physicalTrap?65504:16384;
  const view=new DataView(w.memory.buffer);
  for(let i=0;i<n;i++)view.setBigUint64(input+i*8,floatBits[i%floatBits.length],true);
  let status='ok';try{w.transform(input,n,out,n)}catch(e){assert(e instanceof WebAssembly.RuntimeError);status='trap'}
  const all=Buffer.from(w.memory.buffer);
  if(status==='ok'){
   for(let i=0;i<n;i++){
    const source=view.getFloat64(input+i*8,true);
    const expected=source*0.5+0.25;
    const actual=view.getFloat64(out+i*8,true);
    if(!Number.isNaN(expected))assert(Object.is(actual,expected),`strict f64 result i=${i} n=${n}`);
   }
  }
  return [status,all.toString('hex')];
 }
 let cases=0;
 for(const n of lengths){
  let expectedSum,expectedMap;
  for(const w of engines){
   const actualSum=runSum(w,n);const actualMap=runMap(w,n);
   if(expectedSum)assert.deepEqual(actualSum,expectedSum,`sum n=${n}`);else expectedSum=actualSum;
   if(expectedMap)assert.deepEqual(actualMap,expectedMap,`transform n=${n}`);else expectedMap=actualMap;
  }
  cases+=2;
 }
 for(const w of engines){
  const sum=runSum(w,17,true),map=runMap(w,17,true);
  if(!globalThis.trapExpected){globalThis.trapExpected=[sum,map]}
  else assert.deepEqual([sum,map],globalThis.trapExpected,'physical trap behavior and committed prefix');
 }
 process.stdout.write(String(cases+2));
})().catch(e=>{console.error(e);process.exitCode=1});
"#,
    )
    .expect("write Node oracle");
    let result = Command::new("node")
        .arg(dir.join("run.cjs"))
        .args(paths_names(&dir))
        .output()
        .expect("run Node Wasm oracle");
    let _ = fs::remove_dir_all(&dir);
    assert!(
        result.status.success(),
        "Node Wasm oracle failed:\n{}\n{}",
        String::from_utf8_lossy(&result.stdout),
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "38");
}

fn paths_names(directory: &std::path::Path) -> [std::path::PathBuf; 4] {
    [
        directory.join("baseline-direct.wasm"),
        directory.join("optimized-direct.wasm"),
        directory.join("baseline-wat.wasm"),
        directory.join("optimized-wat.wasm"),
    ]
}
