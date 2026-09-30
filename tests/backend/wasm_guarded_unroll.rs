use super::support::compiler::{optimized_module, verified_artifact};
use calckernel::{
    BoundsMode, EmitWasmOptions, KirConsumer, OverflowMode, emit_wasm_kir_module,
    emit_wat_kir_module,
};
use std::{fs, process::Command};

const SOURCE: &str = r#"
export unsafe fn scan(values: slice<u32>, flags: slice<u32>, n: u32) -> u32 contract {
  requires n <= values.len && n <= flags.len;
  effects read(values), read(flags);
} {
  let best: u32 = 0;
  let chosen: u32 = 0;
  let i: u32 = 0;
  while i < n {
    if flags[i] == 0 && values[i] > best {
      best = values[i];
      chosen = i;
    }
    i = i + 1;
  }
  return chosen;
}
export unsafe fn scatter(values: slice<u32>, flags: slice<u32>, out: slice<u32>, n: u32) -> void contract {
  requires n <= values.len && n <= flags.len && n <= out.len;
  effects read(values), read(flags), readwrite(out);
} {
  let i: u32 = 0;
  while i < n {
    if flags[i] != 0 { out[i] = values[i] + 1; }
    i = i + 1;
  }
}
"#;

fn artifact(source: &str) -> calckernel::KirPassManagerResult {
    optimized_module(
        source,
        3,
        KirConsumer::WebAssembly,
        OverflowMode::Unchecked,
        BoundsMode::Unchecked,
    )
}

#[test]
fn closed_branching_scalar_loop_emits_four_original_header_guards() {
    let optimized = artifact(SOURCE);
    let module = verified_artifact(&optimized);
    let wat = emit_wat_kir_module(module, EmitWasmOptions { opt_level: 3 }).expect("WAT");
    let scan = wat
        .split("(func $scan")
        .nth(1)
        .unwrap()
        .split("(func $scatter")
        .next()
        .unwrap();
    assert_eq!(
        scan.matches("i32.lt_u").count(),
        4,
        "each of the four logical iterations retains its original header guard:\n{scan}"
    );
    wat::parse_str(&wat).expect("unrolled text validates");
}

#[test]
fn guarded_unroll_preserves_tails_unsigned_ties_traps_and_written_prefix() {
    if !super::support::command::node_available() {
        return;
    }
    let optimized = artifact(SOURCE);
    let module = verified_artifact(&optimized);
    let before = emit_wasm_kir_module(module, EmitWasmOptions { opt_level: 0 }).expect("reference");
    let after = emit_wasm_kir_module(module, EmitWasmOptions { opt_level: 3 }).expect("candidate");
    let directory = super::support::temp::temp_dir("ck-wasm-guarded-unroll");
    fs::create_dir_all(&directory).unwrap();
    fs::write(directory.join("before.wasm"), before).unwrap();
    fs::write(directory.join("after.wasm"), after).unwrap();
    fs::write(directory.join("run.cjs"), r#"
const fs=require('node:fs'),assert=require('node:assert/strict');
(async()=>{
 const engines=[];
 for(const p of process.argv.slice(2))engines.push((await WebAssembly.instantiate(fs.readFileSync(p))).instance.exports);
 let cases=0;
 for(const n of [0,1,2,3,4,5,7,8,9,15,16,17,31])for(const mode of [0,1,2,3]){
  let expected;
  for(const w of engines){
   const a=new Uint32Array(w.memory.buffer);a.fill(0x87654321);
   const flags=1024, out=2048,values=mode===1?65528:mode===2?65536:3072;
   for(let i=0;i<32;i++)a[flags/4+i]=mode===2?1:i%3;
   if(mode!==1&&mode!==2)for(let i=0;i<32;i++)a[values/4+i]=i%4===0?0xffffffff:i*91;
   let value,status='ok';try{value=w.scan(values,n,flags,n,n);}catch(e){assert(e instanceof WebAssembly.RuntimeError);status='trap';}
   const scan=[status,value];
   status='ok';try{w.scatter(values,n,flags,n,out,n,n);}catch(e){assert(e instanceof WebAssembly.RuntimeError);status='trap';}
   const result=[scan,status,Buffer.from(w.memory.buffer).toString('hex')];
   if(expected)assert.deepEqual(result,expected,`n=${n} mode=${mode}`);else expected=result;
  }
  cases++;
 }
 process.stdout.write(String(cases));
})().catch(e=>{console.error(e);process.exitCode=1});
"#).unwrap();
    let result = Command::new("node")
        .arg(directory.join("run.cjs"))
        .arg(directory.join("before.wasm"))
        .arg(directory.join("after.wasm"))
        .output()
        .unwrap();
    let _ = fs::remove_dir_all(directory);
    assert!(
        result.status.success(),
        "{}",
        String::from_utf8_lossy(&result.stderr)
    );
    assert_eq!(String::from_utf8(result.stdout).unwrap(), "52");
}

#[test]
fn guarded_unroll_leaves_side_exit_loops_on_the_original_path() {
    let optimized = artifact(
        r#"
export unsafe fn stop(flags: slice<u32>, n: u32) -> u32 contract {
  requires n <= flags.len;
  effects read(flags);
} {
  let i: u32 = 0;
  while i < n { if flags[i] != 0 { break; } i = i + 1; }
  return i;
}
"#,
    );
    let wat = emit_wat_kir_module(
        verified_artifact(&optimized),
        EmitWasmOptions { opt_level: 3 },
    )
    .unwrap();
    assert_eq!(
        wat.matches("i32.lt_u").count(),
        1,
        "side exits are not duplicated"
    );
}
