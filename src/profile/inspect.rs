use std::fmt::{Arguments, Write};

use super::format::{CkProfile, CkProfileCounter, CkProfileSiteKind};
use super::identity::{
    CkProfileCpuPolicy, CkProfileEndianness, CkProfileObjectFormat, CkProfileOptimizationFamily,
    CkProfileTopology,
};
use super::{CkProfileError, hex};

/// Formats deterministic schema-1 JSON for a profile.
///
/// # Errors
///
/// Returns a profile validation error when the embedded identity is invalid.
pub fn inspect_profile_json(profile: &CkProfile) -> Result<String, CkProfileError> {
    let identity_digest = profile.identity.digest()?;
    let identity = &profile.identity;
    let mut output = String::with_capacity(2_048);
    output.push_str("{\"schema\":1,\"format\":\"CKPROF01\",\"identityDigest\":\"");
    output.push_str(&hex(&identity_digest));
    output.push_str("\",\"identity\":{");
    output.push_str("\"compilerPackage\":");
    push_json_string(&mut output, &identity.compiler.package_version);
    append(
        &mut output,
        format_args!(
            ",\"compilerSource\":\"{}\",\"profileRuntime\":\"{}\",\"semanticGraph\":\"{}\",\"preProfileKir\":\"{}\",\"siteTable\":\"{}\"",
            hex(&identity.compiler.source_identity),
            hex(&identity.compiler.profile_runtime_identity),
            hex(&identity.module.semantic_graph_digest),
            hex(&identity.module.pre_profile_kir_digest),
            hex(&identity.module.site_table_digest),
        ),
    );
    append(
        &mut output,
        format_args!(
            ",\"schemas\":{{\"language\":{},\"nativeAbi\":{},\"runtimeAbi\":{},\"kir\":{},\"proof\":{},\"costModel\":{},\"targetProfile\":{},\"llvmBridge\":{},\"cache\":{}}}",
            identity.schemas.language,
            identity.schemas.native_abi,
            identity.schemas.runtime_abi,
            identity.schemas.kir,
            identity.schemas.proof,
            identity.schemas.cost_model,
            identity.schemas.target_profile,
            identity.schemas.llvm_bridge,
            identity.schemas.cache,
        ),
    );
    output.push_str(",\"target\":{\"triple\":");
    push_json_string(&mut output, &identity.target.triple);
    append(
        &mut output,
        format_args!(
            ",\"pointerWidth\":{},\"endianness\":\"{}\",\"objectFormat\":\"{}\",\"osAbi\":",
            identity.target.pointer_width,
            endianness_name(identity.target.endianness),
            object_format_name(identity.target.object_format),
        ),
    );
    push_json_string(&mut output, &identity.target.os_abi);
    append(
        &mut output,
        format_args!(
            ",\"targetSet\":\"{}\"}}",
            hex(&identity.target.target_set_digest)
        ),
    );
    append(
        &mut output,
        format_args!(
            ",\"modes\":{{\"overflowChecked\":{},\"boundsChecked\":{},\"strictFloat\":{},\"sanitizer\":{},\"topology\":\"{}\",\"optimizationFamily\":\"{}\",\"cpuPolicy\":\"{}\"}}",
            identity.modes.overflow_checked,
            identity.modes.bounds_checked,
            identity.modes.strict_float,
            identity.modes.sanitizer,
            topology_name(identity.modes.topology),
            optimization_name(identity.modes.optimization_family),
            cpu_policy_name(identity.modes.cpu_policy),
        ),
    );
    let contract = &identity.contract;
    append(
        &mut output,
        format_args!(
            ",\"contract\":{{\"formatSchema\":{},\"contractSchema\":{},\"inspectionSchema\":{},\"minimumObservations\":{},\"branchDominanceBasisPoints\":{},\"histogramDominanceBasisPoints\":{},\"coldBasisPoints\":{},\"hotCoverageBasisPoints\":{},\"minimumRootWorkBasisPoints\":{},\"minimumVariantBenefitBasisPoints\":{},\"minimumAbsoluteCostUnits\":{},\"maximumEnhancedVariants\":{},\"maximumAdditionalKirBasisPoints\":{},\"maximumSites\":{},\"maximumShards\":{},\"histogramBuckets\":{},\"maximumCandidateConstants\":{},\"maximumProfileBytes\":{}}}}}",
            contract.format_schema,
            contract.contract_schema,
            contract.inspection_schema,
            contract.minimum_decision_observations,
            contract.branch_dominance_basis_points,
            contract.histogram_dominance_basis_points,
            contract.cold_basis_points,
            contract.hot_work_coverage_basis_points,
            contract.minimum_root_work_basis_points,
            contract.minimum_variant_benefit_basis_points,
            contract.minimum_absolute_cost_units,
            contract.maximum_enhanced_variants,
            contract.maximum_additional_kir_basis_points,
            contract.maximum_sites,
            contract.maximum_shards,
            contract.histogram_buckets,
            contract.maximum_candidate_constants,
            contract.maximum_profile_bytes,
        ),
    );
    append(
        &mut output,
        format_args!(
            ",\"compatibleCompilerPackage\":{},\"completedRuns\":{},\"mergedShards\":{},\"sites\":{},\"observedSites\":{},\"saturatedSites\":{},\"overflowed\":{},\"incompleteObservations\":{},\"siteRecords\":[",
            compiler_package_compatible(profile),
            profile.completed_runs,
            profile.merged_shards,
            profile.sites.len(),
            observed_site_count(profile),
            saturated_site_count(profile),
            profile.overflowed,
            profile.incomplete_observations,
        ),
    );
    for (index, (site, counter)) in profile.sites.iter().zip(&profile.counters).enumerate() {
        if index != 0 {
            output.push(',');
        }
        append(
            &mut output,
            format_args!(
                "{{\"id\":\"{}\",\"kind\":\"{}\",\"observed\":{},\"saturated\":{}",
                hex(&site.id.0),
                site_kind_name(&site.kind),
                counter.counter.is_observed(),
                counter.counter.is_saturated(),
            ),
        );
        match &counter.counter {
            CkProfileCounter::Scalar(value) => {
                append(&mut output, format_args!(",\"count\":{value}"));
            }
            CkProfileCounter::Histogram { buckets, .. } => {
                output.push_str(",\"buckets\":[");
                push_u64_list(&mut output, buckets);
                output.push(']');
            }
            CkProfileCounter::CandidateConstant {
                candidates, other, ..
            } => {
                output.push_str(",\"candidates\":[");
                push_u64_list(&mut output, candidates);
                append(&mut output, format_args!("],\"other\":{other}"));
            }
        }
        output.push('}');
    }
    output.push_str("]}\n");
    Ok(output)
}

/// Formats deterministic human-readable inspection for a profile.
///
/// # Errors
///
/// Returns a profile validation error when the embedded identity is invalid.
pub fn inspect_profile_text(profile: &CkProfile) -> Result<String, CkProfileError> {
    let digest = profile.identity.digest()?;
    let mut output = String::new();
    append(&mut output, format_args!("format: CKPROF01\n"));
    append(&mut output, format_args!("identity: {}\n", hex(&digest)));
    append(
        &mut output,
        format_args!(
            "compiler package: {}\n",
            profile.identity.compiler.package_version
        ),
    );
    append(
        &mut output,
        format_args!("target: {}\n", profile.identity.target.triple),
    );
    append(
        &mut output,
        format_args!(
            "topology: {}\n",
            topology_name(profile.identity.modes.topology)
        ),
    );
    append(
        &mut output,
        format_args!(
            "coverage: {}/{}\n",
            observed_site_count(profile),
            profile.sites.len()
        ),
    );
    append(
        &mut output,
        format_args!("completed runs: {}\n", profile.completed_runs),
    );
    append(
        &mut output,
        format_args!("merged shards: {}\n", profile.merged_shards),
    );
    append(
        &mut output,
        format_args!("saturated sites: {}\n", saturated_site_count(profile)),
    );
    append(
        &mut output,
        format_args!("overflowed: {}\n", profile.overflowed),
    );
    append(
        &mut output,
        format_args!(
            "incomplete observations: {}\n",
            profile.incomplete_observations
        ),
    );
    for (site, counter) in profile.sites.iter().zip(&profile.counters) {
        append(
            &mut output,
            format_args!(
                "site {} {} observed={} saturated={}",
                hex(&site.id.0),
                site_kind_name(&site.kind),
                counter.counter.is_observed(),
                counter.counter.is_saturated(),
            ),
        );
        output.push('\n');
    }
    Ok(output)
}

fn observed_site_count(profile: &CkProfile) -> usize {
    profile
        .counters
        .iter()
        .filter(|record| record.counter.is_observed())
        .count()
}

fn saturated_site_count(profile: &CkProfile) -> usize {
    profile
        .counters
        .iter()
        .filter(|record| record.counter.is_saturated())
        .count()
}

fn compiler_package_compatible(profile: &CkProfile) -> bool {
    profile.identity.compiler.package_version == env!("CARGO_PKG_VERSION")
}

fn push_json_string(output: &mut String, value: &str) {
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => {
                append(output, format_args!("\\u{:04x}", u32::from(character)));
            }
            character => output.push(character),
        }
    }
    output.push('"');
}

fn push_u64_list(output: &mut String, values: &[u64]) {
    for (index, value) in values.iter().enumerate() {
        if index != 0 {
            output.push(',');
        }
        append(output, format_args!("{value}"));
    }
}

fn append(output: &mut String, arguments: Arguments<'_>) {
    if output.write_fmt(arguments).is_err() {
        unreachable!("writing to String cannot fail");
    }
}

const fn endianness_name(value: CkProfileEndianness) -> &'static str {
    match value {
        CkProfileEndianness::Little => "little",
        CkProfileEndianness::Big => "big",
    }
}

const fn object_format_name(value: CkProfileObjectFormat) -> &'static str {
    match value {
        CkProfileObjectFormat::Elf => "elf",
        CkProfileObjectFormat::MachO => "mach-o",
        CkProfileObjectFormat::Coff => "coff",
    }
}

const fn topology_name(value: CkProfileTopology) -> &'static str {
    match value {
        CkProfileTopology::NativeExecutable => "native-executable",
        CkProfileTopology::NativeLibrary => "native-library",
    }
}

const fn optimization_name(value: CkProfileOptimizationFamily) -> &'static str {
    match value {
        CkProfileOptimizationFamily::O2 => "o2",
        CkProfileOptimizationFamily::O3 => "o3",
    }
}

const fn cpu_policy_name(value: CkProfileCpuPolicy) -> &'static str {
    match value {
        CkProfileCpuPolicy::Baseline => "baseline",
        CkProfileCpuPolicy::Native => "native",
        CkProfileCpuPolicy::Multiversion => "multiversion",
    }
}

const fn site_kind_name(value: &CkProfileSiteKind) -> &'static str {
    match value {
        CkProfileSiteKind::FunctionEntry => "function-entry",
        CkProfileSiteKind::Edge { .. } => "edge",
        CkProfileSiteKind::LoopTripHistogram { .. } => "loop-trip-histogram",
        CkProfileSiteKind::SliceLengthHistogram { .. } => "slice-length-histogram",
        CkProfileSiteKind::CandidateConstant { .. } => "candidate-constant",
    }
}
