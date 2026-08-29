#![allow(dead_code)]

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Fixture {
    pub local: &'static str,
    pub oracle: &'static str,
}

pub const CORE_SCALAR: Fixture = Fixture {
    local: "examples/core/scalar.ck",
    oracle: "examples/scalar.ck",
};
pub const CORE_EXPLICIT_CASTS: Fixture = Fixture {
    local: "examples/core/explicit_casts.ck",
    oracle: "examples/explicit_casts.ck",
};
pub const APPLICATION_PRICING: Fixture = Fixture {
    local: "examples/applications/pricing.ck",
    oracle: "examples/pricing.ck",
};
pub const APPLICATION_DIJKSTRA: Fixture = Fixture {
    local: "examples/applications/dijkstra.ck",
    oracle: "examples/dijkstra.ck",
};

pub const CHECKED_SCALAR: Fixture = Fixture {
    local: "examples/checked/scalar.ck",
    oracle: "examples/scalar_checked.ck",
};
pub const CHECKED_SCALAR_CONTROL: Fixture = Fixture {
    local: "examples/checked/scalar_control.ck",
    oracle: "examples/scalar_control_checked.ck",
};
pub const CHECKED_SCALAR_CALLS: Fixture = Fixture {
    local: "examples/checked/scalar_calls.ck",
    oracle: "examples/scalar_calls_checked.ck",
};
pub const CHECKED_SCALAR_LOGICAL: Fixture = Fixture {
    local: "examples/checked/scalar_logical.ck",
    oracle: "examples/scalar_logical_checked.ck",
};

pub const LLVM_SCALAR: Fixture = Fixture {
    local: "examples/llvm/scalar.ck",
    oracle: "examples/llvm_scalar.ck",
};
pub const LLVM_CALLS: Fixture = Fixture {
    local: "examples/llvm/calls.ck",
    oracle: "examples/llvm_calls.ck",
};
pub const LLVM_MEMORY: Fixture = Fixture {
    local: "examples/llvm/memory.ck",
    oracle: "examples/llvm_memory.ck",
};
pub const LLVM_CONTROL_FLOW: Fixture = Fixture {
    local: "examples/llvm/control_flow.ck",
    oracle: "examples/llvm_control_flow.ck",
};
pub const LLVM_SHORT_CIRCUIT: Fixture = Fixture {
    local: "examples/llvm/short_circuit.ck",
    oracle: "examples/llvm_short_circuit.ck",
};
pub const LLVM_BOOL: Fixture = Fixture {
    local: "examples/llvm/bool.ck",
    oracle: "examples/llvm_bool.ck",
};

pub const WASM_SCALAR: Fixture = Fixture {
    local: "examples/wasm/scalar.ck",
    oracle: "examples/wasm_scalar.ck",
};
pub const WASM_CALLS: Fixture = Fixture {
    local: "examples/wasm/calls.ck",
    oracle: "examples/wasm_calls.ck",
};
pub const WASM_MEMORY: Fixture = Fixture {
    local: "examples/wasm/memory.ck",
    oracle: "examples/wasm_memory.ck",
};
pub const WASM_CONTROL_FLOW: Fixture = Fixture {
    local: "examples/wasm/control_flow.ck",
    oracle: "examples/wasm_control_flow.ck",
};
pub const WASM_SHORT_CIRCUIT: Fixture = Fixture {
    local: "examples/wasm/short_circuit.ck",
    oracle: "examples/wasm_short_circuit.ck",
};
pub const WASM_F64_ARRAY: Fixture = Fixture {
    local: "examples/wasm/f64_array.ck",
    oracle: "examples/node-wasm-f64-array/f64_array.ck",
};
pub const WASM_F64_AXPY: Fixture = Fixture {
    local: "examples/wasm/f64_axpy.ck",
    oracle: "examples/wasm/f64-axpy/axpy.ck",
};
pub const WASM_F64_SUM: Fixture = Fixture {
    local: "examples/wasm/f64_sum.ck",
    oracle: "examples/wasm/f64-sum/sum.ck",
};
pub const WASM_PRICING_SOA: Fixture = Fixture {
    local: "examples/wasm/pricing_soa.ck",
    oracle: "examples/wasm/pricing-soa/pricing_soa.ck",
};

pub const BENCH_PRICING_HELPERS: Fixture = Fixture {
    local: "benches/fixtures/pricing_helpers.ck",
    oracle: "bench/perf/fixtures/pricing_helpers.ck",
};
pub const BENCH_PRICING_SOA: Fixture = Fixture {
    local: "benches/fixtures/pricing_soa.ck",
    oracle: "bench/perf/fixtures/pricing_soa.ck",
};
pub const BENCH_F64_KERNELS: Fixture = Fixture {
    local: "benches/fixtures/f64_kernels.ck",
    oracle: "bench/perf/fixtures/f64_kernels.ck",
};
pub const F64_EDGES: Fixture = Fixture {
    local: "tests/fixtures/performance/f64_edges.ck",
    oracle: "tests/fixtures/f64_edges.ck",
};

pub const ORACLE_EXAMPLES: &[Fixture] = &[
    CORE_SCALAR,
    CORE_EXPLICIT_CASTS,
    APPLICATION_PRICING,
    APPLICATION_DIJKSTRA,
    CHECKED_SCALAR,
    CHECKED_SCALAR_CONTROL,
    CHECKED_SCALAR_CALLS,
    CHECKED_SCALAR_LOGICAL,
    LLVM_SCALAR,
    LLVM_CALLS,
    LLVM_MEMORY,
    LLVM_CONTROL_FLOW,
    LLVM_SHORT_CIRCUIT,
    LLVM_BOOL,
    WASM_SCALAR,
    WASM_CALLS,
    WASM_MEMORY,
    WASM_CONTROL_FLOW,
    WASM_SHORT_CIRCUIT,
    WASM_F64_ARRAY,
    WASM_F64_AXPY,
    WASM_F64_SUM,
    WASM_PRICING_SOA,
];

pub const BENCHMARK_FIXTURES: &[Fixture] =
    &[BENCH_PRICING_HELPERS, BENCH_PRICING_SOA, BENCH_F64_KERNELS];

pub const LOCAL_ONLY_EXAMPLES: &[&str] = &[
    "examples/core/control_flow.ck",
    "examples/core/void.ck",
    "examples/core/slices.ck",
    "examples/native/hello.ck",
];
