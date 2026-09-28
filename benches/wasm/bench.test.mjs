import assert from 'node:assert/strict';
import { spawnSync } from 'node:child_process';
import { createHash } from 'node:crypto';
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, readdirSync, rmSync, writeFileSync } from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const repoRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '../..');
const runnerPath = path.join(repoRoot, 'benches/wasm/bench.mjs');
const realCompiler = process.env.CKC;

function withTempDir(run) {
  const directory = mkdtempSync(path.join(os.tmpdir(), 'ck-wasm-bench-test-'));
  try {
    return run(directory);
  } finally {
    rmSync(directory, { recursive: true, force: true });
  }
}

function runRunner(args) {
  return spawnSync(process.execPath, [runnerPath, ...args], {
    cwd: repoRoot,
    encoding: 'utf8',
    timeout: 120_000,
  });
}

function makeWrongResultCompiler(directory) {
  // A valid module exporting calc(i64, i64) -> i64 and always returning 0.
  const wasm = Buffer.from([
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00,
    0x01, 0x07, 0x01, 0x60, 0x02, 0x7e, 0x7e, 0x01, 0x7e,
    0x03, 0x02, 0x01, 0x00,
    0x07, 0x08, 0x01, 0x04, 0x63, 0x61, 0x6c, 0x63, 0x00, 0x00,
    0x0a, 0x06, 0x01, 0x04, 0x00, 0x42, 0x00, 0x0b,
  ]);
  return makeCompiler(directory, 'wrong-ckc.mjs', wasm);
}

function wasmSection(id, payload) {
  return Buffer.concat([Buffer.from([id, payload.length]), Buffer.from(payload)]);
}

function encodeU32Leb(value) {
  const bytes = [];
  let remaining = value >>> 0;
  do {
    let byte = remaining & 0x7f;
    remaining >>>= 7;
    if (remaining !== 0) byte |= 0x80;
    bytes.push(byte);
  } while (remaining !== 0);
  return Buffer.from(bytes);
}

function encodeWasmName(name) {
  const bytes = Buffer.from(name, 'utf8');
  return [bytes.length, ...bytes];
}

function makeZeroF64SumModule(includeHeapBase = true) {
  const header = Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
  const type = wasmSection(1, [1, 0x60, 2, 0x7f, 0x7f, 1, 0x7c]);
  const functionSection = wasmSection(3, [1, 0]);
  const memory = wasmSection(5, [1, 0, 1]);
  const global = includeHeapBase ? wasmSection(6, [1, 0x7f, 0, 0x41, 0, 0x0b]) : null;
  const exportsList = [
    includeHeapBase ? 3 : 2,
    ...encodeWasmName('memory'), 0x02, 0x00,
    ...encodeWasmName('sum_f64'), 0x00, 0x00,
  ];
  if (includeHeapBase) exportsList.push(...encodeWasmName('__ck_heap_base'), 0x03, 0x00);
  const exports = wasmSection(7, exportsList);
  const body = [0, 0x44, 0, 0, 0, 0, 0, 0, 0, 0, 0x0b];
  const code = wasmSection(10, [1, body.length, ...body]);
  return Buffer.concat([header, type, functionSection, memory, ...(global ? [global] : []), exports, code]);
}

function makeTrapOnSecondCallModule() {
  const header = Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
  const type = wasmSection(1, [1, 0x60, 2, 0x7e, 0x7e, 1, 0x7e]);
  const functionSection = wasmSection(3, [1, 0]);
  const global = wasmSection(6, [1, 0x7f, 1, 0x41, 0, 0x0b]);
  const exports = wasmSection(7, [1, ...encodeWasmName('calc'), 0x00, 0x00]);
  const body = [
    0x00,
    0x23, 0x00,
    0x41, 0x01,
    0x6a,
    0x24, 0x00,
    0x23, 0x00,
    0x41, 0x02,
    0x46,
    0x04, 0x40,
    0x00,
    0x0b,
    0x42, 0xd4, 0x00,
    0x0b,
  ];
  const code = wasmSection(10, [1, body.length, ...body]);
  return Buffer.concat([header, type, functionSection, global, exports, code]);
}

function makeConstant84CalcModule() {
  const header = Buffer.from([0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00]);
  const type = wasmSection(1, [1, 0x60, 2, 0x7e, 0x7e, 1, 0x7e]);
  const functionSection = wasmSection(3, [1, 0]);
  const exports = wasmSection(7, [1, ...encodeWasmName('calc'), 0x00, 0x00]);
  const body = [0x01, 0x02, 0x7e, 0x42, 0xd4, 0x00, 0x0b];
  const code = wasmSection(10, [1, body.length, ...body]);
  return Buffer.concat([header, type, functionSection, exports, code]);
}

function makeCompiler(directory, fileName, wasm) {
  const digests = { baseline: 'a'.repeat(64), simd128: 'b'.repeat(64) };
  return makeIdentityCompiler(directory, fileName, (feature) => {
    const metadata = {
      schema: 2,
      target: 'wasm32',
      features: feature,
      profile_sha256: digests[feature],
    };
    return appendTargetSections(wasm, [metadata]);
  });
}

function appendTargetSections(wasm, metadataValues, options = {}) {
  const customName = options.customName ?? 'ck.wasm.target';
  const sections = metadataValues.map((metadata) => {
    const text = typeof metadata === 'string' ? metadata : JSON.stringify(metadata);
    const name = Buffer.from(customName, 'utf8');
    const payload = Buffer.concat([encodeU32Leb(name.length), name, Buffer.from(text, 'utf8')]);
    return Buffer.concat([Buffer.from([0]), encodeU32Leb(payload.length), payload]);
  });
  return Buffer.concat([wasm, ...sections]);
}

function makeIdentityCompiler(directory, fileName, wasmForFeature, options = {}) {
  const digests = { baseline: 'a'.repeat(64), simd128: 'b'.repeat(64) };
  const wasmByFeature = Object.fromEntries(
    ['baseline', 'simd128'].map((feature) => [feature, wasmForFeature(feature).toString('base64')]),
  );
  const headerDigest = options.headerDigest ?? ((feature) => digests[feature]);
  const headerDigests = Object.fromEntries(
    ['baseline', 'simd128'].map((feature) => [feature, headerDigest(feature)]),
  );
  const compilerPath = path.join(directory, fileName);
  writeFileSync(compilerPath, `#!/usr/bin/env node
import { writeFileSync } from 'node:fs';
if (process.argv[2] === '--version') {
  process.stdout.write('identity-aware test fixture compiler\\n');
  process.exit(0);
}
const action = process.argv[2];
const args = process.argv.slice(3);
const value = (name) => {
  const index = args.indexOf(name);
  return index < 0 ? null : args[index + 1];
};
const feature = value('--wasm-features');
const optLevel = value('--opt-level');
if (!['baseline', 'simd128'].includes(feature) || !['0', '3'].includes(optLevel)) process.exit(2);
if (value('--overflow') !== 'unchecked' || value('--bounds') !== 'unchecked') process.exit(2);
if (action === 'emit-kir') {
  if (value('--consumer') !== 'wasm') process.exit(2);
  const profileDigest = feature === 'baseline' ? '${headerDigests.baseline}' : '${headerDigests.simd128}';
  process.stdout.write('kir-v3 consumer=wasm overflow=unchecked bounds=unchecked sanitizer=off profile-schema=1 profile-sha256=' + profileDigest + '\\n');
  process.exit(0);
}
if (action !== 'emit-wasm') process.exit(2);
const outputIndex = args.indexOf('--out');
if (outputIndex < 0) process.exit(2);
const wasm = '${wasmByFeature.baseline}';
const selectedWasm = feature === 'baseline' ? wasm : '${wasmByFeature.simd128}';
writeFileSync(args[outputIndex + 1], Buffer.from(selectedWasm, 'base64'));
`);
  chmodSync(compilerPath, 0o755);
  return compilerPath;
}

function expectedBranchDiamond(size) {
  let total = 0n;
  for (let index = 0; index < size; index += 1) {
    const i = BigInt(index);
    total += index % 2 === 0 ? i + 7n : 2n * i + 5n;
    total += index % 3 === 0 ? 13n : 17n;
  }
  return total.toString();
}

function expectedNestedControl(size) {
  let total = 0n;
  for (let outer = 0; outer < size; outer += 1) {
    for (let inner = 0; inner < 10; inner += 1) {
      if (inner === 7) break;
      if ((outer + inner) % 3 === 0) continue;
      total += BigInt(outer + 1) * BigInt(inner + 2);
    }
  }
  return total.toString();
}

function lebBytes(value) {
  const bytes = [];
  let remaining = value >>> 0;
  do {
    let byte = remaining & 0x7f;
    remaining >>>= 7;
    if (remaining !== 0) byte |= 0x80;
    bytes.push(byte);
  } while (remaining !== 0);
  return bytes;
}

function wasmCodeBodies(bytes) {
  assert.deepEqual([...bytes.subarray(0, 8)], [0, 0x61, 0x73, 0x6d, 1, 0, 0, 0]);
  let offset = 8;
  while (offset < bytes.length) {
    const sectionId = bytes[offset++];
    const sectionLength = readLeb(bytes, offset);
    offset = sectionLength.next;
    const sectionEnd = offset + sectionLength.value;
    assert.ok(sectionEnd <= bytes.length, 'Wasm section is truncated');
    if (sectionId === 10) {
      const functionCount = readLeb(bytes, offset);
      offset = functionCount.next;
      const bodies = [];
      for (let functionIndex = 0; functionIndex < functionCount.value; functionIndex += 1) {
        const bodyLength = readLeb(bytes, offset);
        offset = bodyLength.next;
        bodies.push(bytes.subarray(offset, offset + bodyLength.value));
        offset += bodyLength.value;
      }
      assert.equal(offset, sectionEnd, 'Wasm code section has trailing bytes');
      return bodies;
    }
    offset = sectionEnd;
  }
  throw new Error('Wasm module has no code section');
}

function readLeb(bytes, start) {
  let value = 0;
  let shift = 0;
  let offset = start;
  while (offset < bytes.length && shift <= 28) {
    const byte = bytes[offset++];
    value |= (byte & 0x7f) << shift;
    if ((byte & 0x80) === 0) return { value: value >>> 0, next: offset };
    shift += 7;
  }
  throw new Error('Invalid LEB128 value in Wasm module');
}

function hasSimdOpcode(bytes, subopcode) {
  const encodedSubopcode = lebBytes(subopcode);
  return wasmCodeBodies(bytes).some((body) => {
    for (let index = 0; index + encodedSubopcode.length < body.length; index += 1) {
      if (body[index] !== 0xfd) continue;
      if (encodedSubopcode.every((byte, offset) => body[index + offset + 1] === byte)) return true;
    }
    return false;
  });
}

function emitFixture(compilerPath, command, source, feature, optLevel, outPath) {
  const result = spawnSync(compilerPath, [
    command,
    path.join(repoRoot, source),
    '--out', outPath,
    '--wasm-features', feature,
    '--overflow', 'unchecked',
    '--bounds', 'unchecked',
    '--opt-level', String(optLevel),
  ], { cwd: repoRoot, encoding: 'utf8', timeout: 60_000 });
  assert.equal(result.status, 0, `${command} ${source} ${feature} O${optLevel}: ${result.stderr || result.stdout}`);
}

test('rejects invalid benchmark options before starting a compiler', () => {
  const zeroSamples = runRunner(['--samples', '0']);
  assert.notEqual(zeroSamples.status, 0);
  assert.match(zeroSamples.stderr, /samples/i);

  const zeroEmissionSamples = runRunner(['--emission-samples', '0']);
  assert.notEqual(zeroEmissionSamples.status, 0);
  assert.match(zeroEmissionSamples.stderr, /emission-samples/i);

  const unknownOption = runRunner(['--not-a-benchmark-option']);
  assert.notEqual(unknownOption.status, 0);
  assert.match(unknownOption.stderr, /unknown|invalid|usage/i);

  const unknownFeature = runRunner(['--wasm-features', 'relaxed-simd']);
  assert.notEqual(unknownFeature.status, 0);
  assert.match(unknownFeature.stderr, /wasm-features|baseline|simd128/i);
});

test('records requested Wasm feature and canonical target metadata verified against emit-kir', () => {
  withTempDir((directory) => {
    const compilerPath = makeCompiler(directory, 'identity-ckc.mjs', makeConstant84CalcModule());
    const reportPath = path.join(directory, 'identity.json');
    const result = runRunner([
      '--ckc', compilerPath,
      '--out', reportPath,
      '--case', 'scalar-small-call',
      '--wasm-features', 'simd128',
      '--samples', '1',
      '--emission-samples', '2',
      '--warmup', '0',
      '--batch', '1',
      '--size', '3',
    ]);
    assert.equal(result.status, 0, result.stderr || result.stdout);

    const report = JSON.parse(readFileSync(reportPath, 'utf8'));
    assert.equal(report.configuration.wasm_features, 'simd128');
    assert.equal(report.configuration.emission_samples, 2);
    assert.equal(report.results.length, 2);
    for (const row of report.results) {
      assert.deepEqual(row.artifact.target_metadata, {
        schema: 2,
        target: 'wasm32',
        features: 'simd128',
        profile_sha256: 'b'.repeat(64),
      });
      assert.equal(row.artifact.wasm_stats.code_section_bytes, 9);
      assert.equal(row.artifact.wasm_stats.function_count, 1);
      assert.deepEqual(row.artifact.wasm_stats.function_body_bytes, [7]);
      assert.equal(row.artifact.wasm_stats.local_count, 2);
      assert.equal(row.artifact.emission_outputs.length, 2);
      assert.equal(row.samples.ck_emission_raw_ns.length, 2);
      assert.ok(row.timings_ns.ck_kir_profile_evidence >= 0);
      assert.ok(row.timings_ns.ck_emission >= 0);
    }
  });
});

test('rejects a missing or duplicated ck.wasm.target section before report commit', () => {
  const validModule = makeConstant84CalcModule();
  const metadata = {
    schema: 2,
    target: 'wasm32',
    features: 'baseline',
    profile_sha256: 'a'.repeat(64),
  };
  for (const [fileName, module, expectedError] of [
    ['missing-target-ckc.mjs', validModule, /exactly one.*ck\.wasm\.target|ck\.wasm\.target.*exactly one/i],
    ['duplicate-target-ckc.mjs', appendTargetSections(validModule, [metadata, metadata]), /exactly one.*ck\.wasm\.target|ck\.wasm\.target.*exactly one/i],
  ]) {
    withTempDir((directory) => {
      const compilerPath = makeIdentityCompiler(directory, fileName, () => module);
      const reportPath = path.join(directory, 'identity.json');
      const result = runRunner([
        '--ckc', compilerPath,
        '--out', reportPath,
        '--case', 'scalar-small-call',
        '--samples', '1',
        '--warmup', '0',
        '--batch', '1',
        '--size', '3',
      ]);
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, expectedError);
      assert.equal(existsSync(reportPath), false);
      assert.equal(existsSync(path.join(directory, 'artifacts')), false);
    });
  }
});

test('an identity failure leaves a prior report and its referenced artifact intact', () => {
  withTempDir((directory) => {
    const compilerPath = makeIdentityCompiler(
      directory,
      'legacy-ckc.mjs',
      () => makeConstant84CalcModule(),
    );
    const reportPath = path.join(directory, 'report.json');
    const artifactDirectory = path.join(directory, 'artifacts');
    mkdirSync(artifactDirectory);
    const artifactPath = path.join(artifactDirectory, 'prior.wasm');
    const artifactBytes = Buffer.from('previously referenced artifact');
    const previousReport = Buffer.from(JSON.stringify({
      schema_version: 1,
      results: [{ artifact: { path: 'artifacts/prior.wasm' } }],
    }));
    writeFileSync(artifactPath, artifactBytes);
    writeFileSync(reportPath, previousReport);

    const result = runRunner([
      '--ckc', compilerPath,
      '--out', reportPath,
      '--case', 'scalar-small-call',
      '--samples', '1',
      '--warmup', '0',
      '--batch', '1',
      '--size', '3',
    ]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /exactly one.*ck\.wasm\.target/i);
    assert.deepEqual(readFileSync(reportPath), previousReport);
    assert.deepEqual(readFileSync(artifactPath), artifactBytes);
  });
});

test('rejects non-canonical target JSON and a profile digest that disagrees with emit-kir', () => {
  const validModule = makeConstant84CalcModule();
  const canonicalText = JSON.stringify({
    schema: 2,
    target: 'wasm32',
    features: 'baseline',
    profile_sha256: 'a'.repeat(64),
  });
  const nonCanonical = canonicalText.replace('{"schema":2', '{ "schema":2');
  for (const [fileName, module, expectedError] of [
    ['non-canonical-target-ckc.mjs', appendTargetSections(validModule, [nonCanonical]), /canonical|deterministic|JSON/i],
    ['legacy-schema-target-ckc.mjs', appendTargetSections(validModule, [{
      schema: 1,
      target: 'wasm32',
      features: 'baseline',
      profile_sha256: 'a'.repeat(64),
    }]), /schema|invalid/i],
    ['wrong-digest-ckc.mjs', appendTargetSections(validModule, [{
      schema: 2,
      target: 'wasm32',
      features: 'baseline',
      profile_sha256: 'c'.repeat(64),
    }]), /profile.*digest|digest.*profile/i],
    ['wrong-feature-ckc.mjs', appendTargetSections(validModule, [{
      schema: 2,
      target: 'wasm32',
      features: 'baseline',
      profile_sha256: 'a'.repeat(64),
    }]), /feature.*does not match requested/i],
  ]) {
    withTempDir((directory) => {
      const compilerPath = makeIdentityCompiler(directory, fileName, () => module);
      const reportPath = path.join(directory, 'identity.json');
      const result = runRunner([
        '--ckc', compilerPath,
        '--out', reportPath,
        '--case', 'scalar-small-call',
        ...(fileName === 'wrong-feature-ckc.mjs' ? ['--wasm-features', 'simd128'] : []),
        '--samples', '1',
        '--warmup', '0',
        '--batch', '1',
        '--size', '3',
      ]);
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, expectedError);
      assert.equal(existsSync(reportPath), false);
      assert.equal(existsSync(path.join(directory, 'artifacts')), false);
    });
  }
});

test('writes complete O0/O3 runtime evidence for all wasm fixtures', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  assert.ok(readFileSync(realCompiler).byteLength > 0, `missing compiler: ${realCompiler}`);

  withTempDir((directory) => {
    const reportPath = path.join(directory, 'report.json');
    const result = runRunner([
      '--ckc', realCompiler,
      '--out', reportPath,
      '--samples', '2',
      '--warmup', '1',
      '--batch', '2',
      '--size', '31',
    ]);
    assert.equal(result.status, 0, result.stderr || result.stdout);

    const report = JSON.parse(readFileSync(reportPath, 'utf8'));
    assert.equal(report.schema_version, 1);
    assert.equal(report.configuration.samples, 2);
    assert.equal(report.configuration.emission_samples, 3);
    assert.equal(report.configuration.warmup, 1);
    assert.equal(report.configuration.batch, 2);
    assert.equal(report.configuration.size, 31);
    assert.equal(report.configuration.wasm_features, 'baseline');
    assert.deepEqual(report.configuration.opt_levels, [0, 3]);
    assert.ok(report.identity.compiler.path);
    assert.match(report.identity.compiler.sha256, /^[a-f0-9]{64}$/);
    assert.ok(report.identity.runner.node_version);
    assert.ok(report.identity.runner.v8_version);
    assert.match(report.identity.runner_sha256, /^[a-f0-9]{64}$/);

    const expectedFixtures = [
      'scalar-small-call',
      'control_flow',
      'f64_sum',
      'f64_axpy',
      'f64_map',
      'i32_map',
      'u32_compare_select',
      'i32_to_f64',
      'u32_to_f64',
      'u32_alias_map',
      'u32_reduce_sum',
      'u32_reduce_product',
      'u32_cursor_copy',
      'u32_fill',
      'u32_field_offset',
      'pricing_soa',
      'branch_diamond',
      'nested_control',
      'pricing_one_calls',
      'pricing_batch_call',
    ];
    assert.deepEqual([...new Set(report.results.map((entry) => entry.case))].sort(), expectedFixtures.sort());
    assert.equal(report.results.length, expectedFixtures.length * 2);
    for (const optLevel of [0, 3]) {
      const scalarPricing = report.results.find((entry) =>
        entry.case === 'pricing_one_calls' && entry.opt_level === optLevel);
      const batchPricing = report.results.find((entry) =>
        entry.case === 'pricing_batch_call' && entry.opt_level === optLevel);
      assert.equal(scalarPricing.correctness.actual.outputs.out_totals.sha256,
        batchPricing.correctness.actual.outputs.out_totals.sha256);
      assert.deepEqual(scalarPricing.workload, {
        logical_rows_per_workload: 31,
        wasm_calls_per_workload: 31,
        benchmark_workloads_per_sample: 2,
      });
      assert.deepEqual(batchPricing.workload, {
        logical_rows_per_workload: 31,
        wasm_calls_per_workload: 1,
        benchmark_workloads_per_sample: 2,
      });
    }
    const fillO0 = report.results.find((entry) => entry.case === 'u32_fill' && entry.opt_level === 0);
    const fillO3 = report.results.find((entry) => entry.case === 'u32_fill' && entry.opt_level === 3);
    assert.equal(fillO0.correctness.actual.outputs.dst.sha256, fillO3.correctness.actual.outputs.dst.sha256);
    assert.deepEqual(fillO0.workload, {
      elements_per_workload: 31,
      wasm_calls_per_workload: 1,
      benchmark_workloads_per_sample: 2,
    });

    for (const entry of report.results) {
      assert.ok([0, 3].includes(entry.opt_level));
      assert.equal(entry.correctness.status, 'passed');
      assert.deepEqual(entry.correctness.actual, entry.correctness.expected);
      assert.ok(entry.source.path.endsWith('.ck'));
      assert.match(entry.source.sha256, /^[a-f0-9]{64}$/);
      assert.ok(entry.artifact.bytes > 0);
      assert.match(entry.artifact.sha256, /^[a-f0-9]{64}$/);
      assert.deepEqual(Object.keys(entry.artifact.target_metadata), [
        'schema', 'target', 'features', 'profile_sha256',
      ]);
      assert.equal(entry.artifact.target_metadata.schema, 2);
      assert.equal(entry.artifact.target_metadata.target, 'wasm32');
      assert.equal(entry.artifact.target_metadata.features, 'baseline');
      assert.match(entry.artifact.target_metadata.profile_sha256, /^[a-f0-9]{64}$/);
      assert.match(entry.artifact.path, /^artifacts\/.+\.wasm$/);
      assert.ok(entry.artifact.wasm_stats.code_section_bytes > 0);
      assert.ok(entry.artifact.wasm_stats.function_count > 0);
      assert.equal(entry.artifact.wasm_stats.function_body_bytes.length,
        entry.artifact.wasm_stats.function_count);
      assert.ok(entry.artifact.wasm_stats.function_body_bytes.every((size) => size > 0));
      assert.ok(entry.artifact.wasm_stats.local_count >= 0);
      assert.equal(entry.artifact.emission_outputs.length, 3);
      assert.equal(entry.samples.ck_emission_raw_ns.length, 3);
      const artifactBytes = readFileSync(path.join(path.dirname(reportPath), entry.artifact.path));
      assert.equal(artifactBytes.byteLength, entry.artifact.bytes);
      assert.equal(createHash('sha256').update(artifactBytes).digest('hex'), entry.artifact.sha256);
      assert.equal(entry.samples.warmup.length, 1);
      assert.equal(entry.samples.kernel_ns.length, 2);
      assert.equal(entry.samples.host_preparation_ns.length, 2);
      assert.equal(entry.samples.readback_ns.length, 2);
      assert.equal(entry.samples.round_end_to_end_ns.length, 2);
      assert.ok(entry.timings_ns.ck_emission >= 0);
      assert.ok(entry.timings_ns.module_compile_first_in_process >= 0);
      assert.ok(entry.timings_ns.instantiate_first_in_process >= 0);
      for (const samples of Object.values(entry.samples)) {
        assert.ok(samples.every((sample) => Number.isFinite(sample) && sample >= 0));
      }
    }
  });
});

test('u32 cursor-copy case matches scalar at loop boundaries', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  for (const size of [0, 1, 3, 4, 5, 1025]) {
    withTempDir((directory) => {
      const reportPath = path.join(directory, `cursor-${size}.json`);
      const result = runRunner([
        '--ckc', realCompiler,
        '--out', reportPath,
        '--case', 'u32_cursor_copy',
        '--samples', '1',
        '--warmup', '0',
        '--batch', '3',
        '--size', String(size),
      ]);
      assert.equal(result.status, 0, `size=${size}: ${result.stderr || result.stdout}`);
      const report = JSON.parse(readFileSync(reportPath, 'utf8'));
      assert.equal(report.results.length, 2);
      for (const row of report.results) {
        assert.equal(row.correctness.status, 'passed');
        assert.deepEqual(row.correctness.actual, row.correctness.expected);
      }
    });
  }
});

test('u32 field-offset case matches O0 at empty and populated sizes', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  for (const size of [0, 1, 4]) {
    withTempDir((directory) => {
      const result = runRunner([
        '--ckc', realCompiler,
        '--out', path.join(directory, `field-${size}.json`),
        '--case', 'u32_field_offset',
        '--samples', '1',
        '--warmup', '0',
        '--batch', '3',
        '--size', String(size),
      ]);
      assert.equal(result.status, 0, `size=${size}: ${result.stderr || result.stdout}`);
    });
  }
});

test('modular u32 reductions match scalar at vector boundaries and wrap', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  for (const size of [0, 3, 4, 5, 1025]) {
    withTempDir((directory) => {
      const reportPath = path.join(directory, `reduction-${size}.json`);
      const result = runRunner([
        '--ckc', realCompiler,
        '--out', reportPath,
        '--case', 'u32_reduce_sum',
        '--case', 'u32_reduce_product',
        '--wasm-features', 'simd128',
        '--samples', '1',
        '--warmup', '0',
        '--batch', '3',
        '--size', String(size),
      ]);
      assert.equal(result.status, 0, `size=${size}: ${result.stderr || result.stdout}`);
      const report = JSON.parse(readFileSync(reportPath, 'utf8'));
      assert.equal(report.results.length, 4);
      for (const row of report.results) {
        assert.equal(row.correctness.status, 'passed');
        assert.deepEqual(row.correctness.actual, row.correctness.expected);
      }
    });
  }
});

test('modular reductions use four SIMD lanes and scalar modular fold', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  withTempDir((directory) => {
    const source = 'examples/wasm/reduction.ck';
    const baselinePath = path.join(directory, 'baseline.wat');
    const simdO0Path = path.join(directory, 'simd-O0.wat');
    const simdO3Path = path.join(directory, 'simd-O3.wat');
    const wasmPath = path.join(directory, 'simd-O3.wasm');
    emitFixture(realCompiler, 'emit-wat', source, 'baseline', 3, baselinePath);
    emitFixture(realCompiler, 'emit-wat', source, 'simd128', 0, simdO0Path);
    emitFixture(realCompiler, 'emit-wat', source, 'simd128', 3, simdO3Path);
    emitFixture(realCompiler, 'emit-wasm', source, 'simd128', 3, wasmPath);
    for (const scalarPath of [baselinePath, simdO0Path]) {
      assert.doesNotMatch(readFileSync(scalarPath, 'utf8'), /\bv128\b|\bi32x4\./);
    }
    const vectorWat = readFileSync(simdO3Path, 'utf8');
    for (const lane of [0, 1, 2, 3]) {
      assert.match(vectorWat, new RegExp(`\\bi32x4\\.extract_lane ${lane}\\b`));
    }
    assert.match(vectorWat, /\bv128\.load\b/);
    assert.ok(WebAssembly.validate(readFileSync(wasmPath)));
  });
});

test('unknown-alias u32 map benchmark matches scalar around vector boundaries', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  for (const size of [0, 1, 3, 4, 5, 1025]) {
    withTempDir((directory) => {
      const reportPath = path.join(directory, `alias-${size}.json`);
      const result = runRunner([
        '--ckc', realCompiler,
        '--out', reportPath,
        '--case', 'u32_alias_map',
        '--wasm-features', 'simd128',
        '--samples', '1',
        '--warmup', '0',
        '--batch', '3',
        '--size', String(size),
      ]);
      assert.equal(result.status, 0, `size=${size}: ${result.stderr || result.stdout}`);
      const report = JSON.parse(readFileSync(reportPath, 'utf8'));
      assert.equal(report.results.length, 2);
      for (const row of report.results) {
        assert.equal(row.correctness.status, 'passed');
        assert.deepEqual(row.correctness.actual, row.correctness.expected);
      }
    });
  }
});

test('unknown-alias SIMD128 map checks address ranges before vector loads', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  withTempDir((directory) => {
    const source = 'examples/wasm/alias_map.ck';
    const baselinePath = path.join(directory, 'baseline.wat');
    const simdO0Path = path.join(directory, 'simd-O0.wat');
    const simdO3Path = path.join(directory, 'simd-O3.wat');
    const wasmPath = path.join(directory, 'simd-O3.wasm');
    emitFixture(realCompiler, 'emit-wat', source, 'baseline', 3, baselinePath);
    emitFixture(realCompiler, 'emit-wat', source, 'simd128', 0, simdO0Path);
    emitFixture(realCompiler, 'emit-wat', source, 'simd128', 3, simdO3Path);
    emitFixture(realCompiler, 'emit-wasm', source, 'simd128', 3, wasmPath);
    for (const scalarPath of [baselinePath, simdO0Path]) {
      assert.doesNotMatch(readFileSync(scalarPath, 'utf8'), /\bv128\b|\bi32x4\./);
    }
    const vectorWat = readFileSync(simdO3Path, 'utf8');
    assert.match(vectorWat, /\bi64\.extend_i32_u\b/);
    assert.match(vectorWat, /\bv128\.load\b/);
    assert.match(vectorWat, /\bv128\.store\b/);
    assert.ok(WebAssembly.validate(readFileSync(wasmPath)));
  });
});

test('exact two-lane integer casts match scalar at vector boundaries', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  for (const size of [0, 1, 2, 3, 1025]) {
    withTempDir((directory) => {
      const reportPath = path.join(directory, `cast-${size}.json`);
      const result = runRunner([
        '--ckc', realCompiler,
        '--out', reportPath,
        '--case', 'i32_to_f64',
        '--case', 'u32_to_f64',
        '--wasm-features', 'simd128',
        '--samples', '1',
        '--warmup', '0',
        '--batch', '3',
        '--size', String(size),
      ]);
      assert.equal(result.status, 0, `size=${size}: ${result.stderr || result.stdout}`);
      const report = JSON.parse(readFileSync(reportPath, 'utf8'));
      assert.equal(report.results.length, 4);
      for (const row of report.results) {
        assert.equal(row.correctness.status, 'passed');
        assert.deepEqual(row.correctness.actual, row.correctness.expected);
      }
    });
  }
});

test('two-lane integer casts use exact load64_zero and signed or unsigned SIMD conversion', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  withTempDir((directory) => {
    const source = 'examples/wasm/cast_map.ck';
    const baselinePath = path.join(directory, 'baseline.wat');
    const simdO0Path = path.join(directory, 'simd-O0.wat');
    const simdO3Path = path.join(directory, 'simd-O3.wat');
    const wasmPath = path.join(directory, 'simd-O3.wasm');
    emitFixture(realCompiler, 'emit-wat', source, 'baseline', 3, baselinePath);
    emitFixture(realCompiler, 'emit-wat', source, 'simd128', 0, simdO0Path);
    emitFixture(realCompiler, 'emit-wat', source, 'simd128', 3, simdO3Path);
    emitFixture(realCompiler, 'emit-wasm', source, 'simd128', 3, wasmPath);
    for (const scalarPath of [baselinePath, simdO0Path]) {
      assert.doesNotMatch(readFileSync(scalarPath, 'utf8'), /\bv128\b|\bf64x2\./);
    }
    const vectorWat = readFileSync(simdO3Path, 'utf8');
    assert.match(vectorWat, /\bv128\.load64_zero\b/);
    assert.match(vectorWat, /\bf64x2\.convert_low_i32x4_s\b/);
    assert.match(vectorWat, /\bf64x2\.convert_low_i32x4_u\b/);
    assert.match(vectorWat, /\bv128\.store\b/);
    assert.ok(WebAssembly.validate(readFileSync(wasmPath)));
  });
});

test('u32 compare/select case checks mixed lanes, wrap and scalar tails', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  for (const size of [0, 3, 4, 5, 1025]) {
    withTempDir((directory) => {
      const reportPath = path.join(directory, `compare-${size}.json`);
      const result = runRunner([
        '--ckc', realCompiler,
        '--out', reportPath,
        '--case', 'u32_compare_select',
        '--wasm-features', 'simd128',
        '--samples', '1',
        '--warmup', '0',
        '--batch', '3',
        '--size', String(size),
      ]);
      assert.equal(result.status, 0, `size=${size}: ${result.stderr || result.stdout}`);
      const report = JSON.parse(readFileSync(reportPath, 'utf8'));
      assert.deepEqual(report.configuration.cases, ['u32_compare_select']);
      assert.equal(report.results.length, 2);
      for (const row of report.results) {
        assert.equal(row.correctness.status, 'passed');
        assert.deepEqual(row.correctness.actual, row.correctness.expected);
      }
    });
  }
});

test('u32 compare/select emits only selected SIMD128 comparison and bitselect', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  withTempDir((directory) => {
    const source = 'examples/wasm/compare_select.ck';
    const baselineWatPath = path.join(directory, 'baseline.wat');
    const simdO0WatPath = path.join(directory, 'simd-O0.wat');
    const simdO3WatPath = path.join(directory, 'simd-O3.wat');
    const simdO3WasmPath = path.join(directory, 'simd-O3.wasm');
    emitFixture(realCompiler, 'emit-wat', source, 'baseline', 3, baselineWatPath);
    emitFixture(realCompiler, 'emit-wat', source, 'simd128', 0, simdO0WatPath);
    emitFixture(realCompiler, 'emit-wat', source, 'simd128', 3, simdO3WatPath);
    emitFixture(realCompiler, 'emit-wasm', source, 'simd128', 3, simdO3WasmPath);

    for (const scalarPath of [baselineWatPath, simdO0WatPath]) {
      assert.doesNotMatch(readFileSync(scalarPath, 'utf8'), /\bv128\b|\bi32x4\./);
    }
    const vectorWat = readFileSync(simdO3WatPath, 'utf8');
    assert.match(vectorWat, /\bi32x4\.lt_u\b/);
    assert.match(vectorWat, /\bv128\.bitselect\b/);
    assert.match(vectorWat, /\bv128\.load\b/);
    assert.match(vectorWat, /\bv128\.store\b/);
    assert.ok(WebAssembly.validate(readFileSync(simdO3WasmPath)));
  });
});

test('slice maps match scalar IEEE/modular expectations at vector boundary sizes and preserve sentinels', { timeout: 300_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  const sizes = [0, 1, 2, 3, 4, 5, 7, 8, 9, 15, 16, 17, 31, 32, 33, 1024, 1025];
  for (const wasmFeatures of ['baseline', 'simd128']) for (const size of sizes) {
    withTempDir((directory) => {
      const reportPath = path.join(directory, `maps-${wasmFeatures}-${size}.json`);
      const result = runRunner([
        '--ckc', realCompiler,
        '--out', reportPath,
        '--case', 'f64_map',
        '--case', 'i32_map',
        '--wasm-features', wasmFeatures,
        '--samples', '1',
        '--warmup', '0',
        '--batch', '4',
        '--size', String(size),
      ]);
      assert.equal(result.status, 0, `size=${size}: ${result.stderr || result.stdout}`);
      const report = JSON.parse(readFileSync(reportPath, 'utf8'));
      assert.deepEqual(report.configuration.cases, ['f64_map', 'i32_map']);
      assert.equal(report.configuration.wasm_features, wasmFeatures);
      assert.equal(report.configuration.size, size);
      assert.equal(report.results.length, 4);
      for (const row of report.results) {
        assert.equal(row.correctness.status, 'passed');
        assert.deepEqual(row.correctness.actual, row.correctness.expected);
      }
    });
  }
});

test('baseline and O0 maps stay scalar while SIMD128 O3 WAT and Wasm use lane operations', { timeout: 180_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  const fixtures = [
    { source: 'examples/wasm/f64_map.ck', operations: ['f64x2.mul', 'f64x2.add'], opcodes: [0xf2, 0xf0], align: 8 },
    { source: 'examples/wasm/i32_map.ck', operations: ['i32x4.mul', 'i32x4.add'], opcodes: [0xb5, 0xae], align: 4 },
  ];
  withTempDir((directory) => {
    for (const fixture of fixtures) {
      const baseName = path.basename(fixture.source, '.ck');
      const baselineWatPath = path.join(directory, `${baseName}-baseline-O3.wat`);
      const simdO0WatPath = path.join(directory, `${baseName}-simd-O0.wat`);
      const simdO3WatPath = path.join(directory, `${baseName}-simd-O3.wat`);
      const simdO3WasmPath = path.join(directory, `${baseName}-simd-O3.wasm`);
      emitFixture(realCompiler, 'emit-wat', fixture.source, 'baseline', 3, baselineWatPath);
      emitFixture(realCompiler, 'emit-wat', fixture.source, 'simd128', 0, simdO0WatPath);
      emitFixture(realCompiler, 'emit-wat', fixture.source, 'simd128', 3, simdO3WatPath);
      emitFixture(realCompiler, 'emit-wasm', fixture.source, 'simd128', 3, simdO3WasmPath);

      const scalarPattern = /\bv128\b|(?:f64x2|i32x4)\./;
      assert.doesNotMatch(readFileSync(baselineWatPath, 'utf8'), scalarPattern, `${baseName} baseline O3`);
      assert.doesNotMatch(readFileSync(simdO0WatPath, 'utf8'), scalarPattern, `${baseName} SIMD128 O0`);
      const vectorWat = readFileSync(simdO3WatPath, 'utf8');
      for (const operation of fixture.operations) {
        assert.match(vectorWat, new RegExp(`\\b${operation.replace('.', '\\.')}\\b`), `${baseName} SIMD128 O3 WAT`);
      }
      assert.match(vectorWat, /v128\.load/);
      assert.match(vectorWat, /v128\.store/);
      assert.match(vectorWat, new RegExp(`v128\\.load offset=0 align=${fixture.align}`));
      assert.match(vectorWat, new RegExp(`v128\\.store offset=0 align=${fixture.align}`));

      const wasm = readFileSync(simdO3WasmPath);
      assert.ok(WebAssembly.validate(wasm), `${baseName} SIMD128 O3 binary validates`);
      for (const opcode of fixture.opcodes) {
        assert.ok(hasSimdOpcode(wasm, opcode), `${baseName} SIMD128 O3 binary contains SIMD opcode 0x${opcode.toString(16)}`);
      }
    }
  });
});

test('checks branch diamonds and nested break/continue at edge sizes', { timeout: 120_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  for (const size of [0, 1, 31]) {
    withTempDir((directory) => {
      const reportPath = path.join(directory, `edge-${size}.json`);
      const result = runRunner([
        '--ckc', realCompiler,
        '--out', reportPath,
        '--case', 'branch_diamond',
        '--case', 'nested_control',
        '--samples', '1',
        '--warmup', '0',
        '--batch', '1',
        '--size', String(size),
      ]);
      assert.equal(result.status, 0, `size=${size}: ${result.stderr || result.stdout}`);
      const report = JSON.parse(readFileSync(reportPath, 'utf8'));
      assert.deepEqual(report.configuration.cases, ['branch_diamond', 'nested_control']);
      assert.equal(report.configuration.size, size);
      assert.equal(report.results.length, 4);
      for (const row of report.results) {
        const expected = row.case === 'branch_diamond'
          ? expectedBranchDiamond(size)
          : expectedNestedControl(size);
        assert.equal(row.correctness.status, 'passed');
        assert.deepEqual(row.correctness.expected.return_values, [expected]);
        assert.deepEqual(row.correctness.actual.return_values, [expected]);
      }
    });
  }
});

test('records the new control-flow workloads at the 1024-element performance size', { timeout: 120_000 }, () => {
  assert.ok(realCompiler, 'set CKC to the absolute path of the compiler build under test');
  withTempDir((directory) => {
    const reportPath = path.join(directory, 'performance.json');
    const result = runRunner([
      '--ckc', realCompiler,
      '--out', reportPath,
      '--case', 'branch_diamond',
      '--case', 'nested_control',
      '--samples', '2',
      '--warmup', '1',
      '--batch', '2',
      '--size', '1024',
    ]);
    assert.equal(result.status, 0, result.stderr || result.stdout);
    const report = JSON.parse(readFileSync(reportPath, 'utf8'));
    assert.equal(report.configuration.size, 1024);
    assert.equal(report.results.length, 4);
    for (const row of report.results) {
      const expected = row.case === 'branch_diamond'
        ? expectedBranchDiamond(1024)
        : expectedNestedControl(1024);
      assert.deepEqual(row.correctness.expected.return_values, [expected, expected]);
      assert.deepEqual(row.correctness.actual.return_values, [expected, expected]);
      assert.equal(row.samples.warmup.length, 1);
      assert.equal(row.samples.kernel_ns.length, 2);
      assert.equal(row.samples.kernel_ns_per_call.length, 2);
    }
  });
});

test('rejects a zero-return f64_sum module for the size-97 cancellation case', () => {
  withTempDir((directory) => {
    const compilerPath = makeCompiler(directory, 'zero-sum-ckc.mjs', makeZeroF64SumModule());
    const reportPath = path.join(directory, 'zero-sum.json');
    const result = runRunner([
      '--ckc', compilerPath,
      '--out', reportPath,
      '--case', 'f64_sum',
      '--samples', '1',
      '--warmup', '0',
      '--batch', '1',
      '--size', '97',
    ]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /correctness mismatch/i);
    assert.equal(existsSync(reportPath), false);
  });
});

test('rejects a memory module without the compiler heap-base global', () => {
  withTempDir((directory) => {
    const compilerPath = makeCompiler(directory, 'missing-heap-base-ckc.mjs', makeZeroF64SumModule(false));
    const result = runRunner([
      '--ckc', compilerPath,
      '--out', path.join(directory, 'missing-heap-base.json'),
      '--case', 'f64_sum',
      '--samples', '1',
      '--warmup', '0',
      '--batch', '1',
      '--size', '97',
    ]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /__ck_heap_base/i);
  });
});

test('a failed rerun preserves the previous report and every artifact it references', { timeout: 120_000 }, () => {
  withTempDir((directory) => {
    const reportPath = path.join(directory, 'report.json');
    const artifactDirectory = path.join(directory, 'artifacts');
    mkdirSync(artifactDirectory);
    const oldArtifacts = [];
    for (const optLevel of [0, 3]) {
      const relativePath = `artifacts/scalar-small-call-O${optLevel}.wasm`;
      const bytes = Buffer.from(`old O${optLevel} artifact`);
      writeFileSync(path.join(directory, relativePath), bytes);
      oldArtifacts.push({ path: relativePath, bytes, sha256: createHash('sha256').update(bytes).digest('hex') });
    }
    const oldReport = {
      schema_version: 1,
      results: oldArtifacts.map((artifact, index) => ({
        case: 'scalar-small-call',
        opt_level: index === 0 ? 0 : 3,
        artifact: { path: artifact.path, bytes: artifact.bytes.byteLength, sha256: artifact.sha256 },
      })),
    };
    const oldReportBytes = Buffer.from(`${JSON.stringify(oldReport)}\n`);
    writeFileSync(reportPath, oldReportBytes);

    const trapCompiler = makeCompiler(directory, 'trap-ckc.mjs', makeTrapOnSecondCallModule());
    const result = runRunner([
      '--ckc', trapCompiler,
      '--out', reportPath,
      '--case', 'scalar-small-call',
      '--samples', '1',
      '--warmup', '1',
      '--batch', '1',
      '--size', '3',
    ]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /unreachable|trap/i);
    assert.deepEqual(readFileSync(reportPath), oldReportBytes);
    for (const artifact of oldArtifacts) {
      const currentBytes = readFileSync(path.join(directory, artifact.path));
      assert.deepEqual(currentBytes, artifact.bytes);
      assert.equal(createHash('sha256').update(currentBytes).digest('hex'), artifact.sha256);
    }
  });
});

test('report commit failure preserves the old report and content it references', { timeout: 120_000, skip: process.getuid?.() === 0 }, () => {
  withTempDir((directory) => {
    const outputDirectory = path.join(directory, 'readonly-output');
    const artifactDirectory = path.join(outputDirectory, 'artifacts');
    mkdirSync(artifactDirectory, { recursive: true });
    const reportPath = path.join(outputDirectory, 'report.json');
    const oldArtifacts = [];
    for (const optLevel of [0, 3]) {
      const relativePath = `artifacts/scalar-small-call-O${optLevel}.wasm`;
      const bytes = Buffer.from(`prior artifact O${optLevel}`);
      writeFileSync(path.join(outputDirectory, relativePath), bytes);
      oldArtifacts.push({ path: relativePath, bytes, sha256: createHash('sha256').update(bytes).digest('hex') });
    }
    const oldReport = {
      schema_version: 1,
      results: oldArtifacts.map((artifact, index) => ({
        case: 'scalar-small-call',
        opt_level: index === 0 ? 0 : 3,
        artifact: { path: artifact.path, bytes: artifact.bytes.byteLength, sha256: artifact.sha256 },
      })),
    };
    const oldReportBytes = Buffer.from(`${JSON.stringify(oldReport)}\n`);
    writeFileSync(reportPath, oldReportBytes);
    chmodSync(reportPath, 0o444);
    chmodSync(artifactDirectory, 0o777);
    chmodSync(outputDirectory, 0o555);

    try {
      const compilerPath = makeCompiler(directory, 'correct-ckc.mjs', makeConstant84CalcModule());
      const result = runRunner([
        '--ckc', compilerPath,
        '--out', reportPath,
        '--case', 'scalar-small-call',
        '--samples', '1',
        '--warmup', '0',
        '--batch', '1',
        '--size', '3',
      ]);
      assert.notEqual(result.status, 0);
      assert.match(result.stderr, /permission|read-only|eacces|report/i);
      assert.deepEqual(readFileSync(reportPath), oldReportBytes);
      for (const artifact of oldArtifacts) {
        const currentBytes = readFileSync(path.join(outputDirectory, artifact.path));
        assert.deepEqual(currentBytes, artifact.bytes);
        assert.equal(createHash('sha256').update(currentBytes).digest('hex'), artifact.sha256);
      }
    } finally {
      chmodSync(outputDirectory, 0o755);
      chmodSync(reportPath, 0o644);
    }
  });
});

test('atomic report commit removes its temporary file and preserves the prior report if rename fails', async () => {
  const { writeReportAtomically } = await import('./bench.mjs');
  assert.equal(typeof writeReportAtomically, 'function');

  withTempDir((directory) => {
    const reportPath = path.join(directory, 'report.json');
    const artifactPath = path.join(directory, 'prior.wasm');
    const priorReport = Buffer.from('{"run":"prior"}\n');
    const priorArtifact = Buffer.from('prior artifact bytes');
    writeFileSync(reportPath, priorReport);
    writeFileSync(artifactPath, priorArtifact);

    assert.throws(
      () => writeReportAtomically(reportPath, '{"run":"new"}\n', () => {
        throw new Error('injected rename failure');
      }),
      /injected rename failure/,
    );
    assert.deepEqual(readFileSync(reportPath), priorReport);
    assert.deepEqual(readFileSync(artifactPath), priorArtifact);
    assert.deepEqual(readdirSync(directory).sort(), ['prior.wasm', 'report.json']);
  });
});

test('exits nonzero and leaves no report when a module returns an unexpected result', () => {
  withTempDir((directory) => {
    const compilerPath = makeWrongResultCompiler(directory);
    const reportPath = path.join(directory, 'mismatch.json');
    const result = runRunner([
      '--ckc', compilerPath,
      '--out', reportPath,
      '--case', 'scalar-small-call',
      '--samples', '1',
      '--warmup', '0',
      '--batch', '1',
      '--size', '3',
    ]);
    assert.notEqual(result.status, 0);
    assert.match(result.stderr, /correctness|expected|mismatch/i);
    assert.equal(existsSync(reportPath), false);
    assert.equal(existsSync(path.join(directory, 'artifacts')), false);
  });
});
