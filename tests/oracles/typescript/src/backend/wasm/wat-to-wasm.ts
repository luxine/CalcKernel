import wabtFactory from "wabt";

const wabt = await wabtFactory();

export function compileWatToWasm(wat: string, fileName = "module.wat"): Uint8Array {
  let module;
  try {
    module = wabt.parseWat(fileName, wat);
    module.resolveNames();
    module.validate();
    return module.toBinary({}).buffer;
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error);
    throw new Error(`WAT to WASM failed: ${message}`);
  } finally {
    module?.destroy();
  }
}
