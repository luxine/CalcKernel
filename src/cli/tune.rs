use std::{fs, path::Path};

#[cfg(feature = "native-toolchain")]
use std::{
    collections::{BTreeMap, BTreeSet},
    ffi::OsString,
    path::PathBuf,
    time::Instant,
};

use calckernel::{decode_tune_decision, inspect_tune_json, inspect_tune_text};

#[cfg(feature = "native-toolchain")]
use calckernel::{
    CandidateOutcome, MeasurementChannel, MeasurementRun, MeasurementScheduler, NativeArtifactKind,
    NativeArtifactPaths, NativeCpu, NativePlatform, NativeTarget, NonPublishableTuneTrial,
    PublicationSet, RoundSummary, SelectionEntrant, TuneArtifactKind, TuneArtifactRole, TuneBudget,
    TuneCache, TuneCacheDomain, TuneCase, TuneDecisionBuildInput, TuneDecisionCandidate,
    TuneDecisionIdentity, TuneDecisionOutput, TuneInvocation, TuneManifest, TunePublishArtifacts,
    TuneRecordedCacheOrigin, TuneRunner, TuneTrialBuildRequest, TuningPlan, assemble_decision,
    calibrate_cases, canonical_frontier_digest, capture_workload, compile_tune_trial,
    derive_round_summary, derive_search_entrants, derive_selection, derive_tune_session_digest,
    encode_completed_tune_decision, enumerate_tuning_space, run_deterministic_search,
    select_size_valid_finalists, verify_tune_trials_with_source,
};
#[cfg(feature = "native-toolchain")]
use sha2::{Digest, Sha256};

#[cfg(feature = "native-toolchain")]
use super::{
    args::{
        ArtifactKind, ParsedArgs, parse_bounds_mode, parse_opt_level, parse_overflow_mode,
        require_input, require_out,
    },
    commands::{
        NativeBuildProduct, compile_replayed_native_build, compile_verified_native_product,
        compiler_source_identity, publish_verified_native_build,
    },
};

pub(super) fn run(args: &[String]) -> Result<(), String> {
    let Some(subcommand) = args.first().map(String::as_str) else {
        return Err("Usage: ckc tune <build|inspect> ...".to_string());
    };
    match subcommand {
        "inspect" => run_inspect(&args[1..]),
        "build" => {
            #[cfg(feature = "native-toolchain")]
            {
                let parsed = ParsedArgs::parse("tune-build", &args[1..])?;
                run_build(&parsed)
            }
            #[cfg(not(feature = "native-toolchain"))]
            {
                let _ = &args[1..];
                Err(super::commands::native_unavailable_error())
            }
        }
        _ => Err(format!("Unknown tune command: {subcommand}.")),
    }
}

fn run_inspect(args: &[String]) -> Result<(), String> {
    let (path, json) = match args {
        [path] => (path, false),
        [path, flag] if flag == "--json" => (path, true),
        _ => return Err("Usage: ckc tune inspect <decision.cktune> [--json]".to_string()),
    };
    let bytes = fs::read(Path::new(path))
        .map_err(|error| format!("read tuning decision {path}: {error}"))?;
    let decision = decode_tune_decision(&bytes).map_err(|error| error.to_string())?;
    let inspection = if json {
        inspect_tune_json(&decision)
    } else {
        inspect_tune_text(&decision)
    }
    .map_err(|error| error.to_string())?;
    print!("{inspection}");
    Ok(())
}

/// Replays one validated tuning decision through the current verified Native
/// pipeline.  No mismatch falls back to an ordinary build.
#[cfg(feature = "native-toolchain")]
pub(super) fn run_replay(args: &ParsedArgs) -> Result<(), String> {
    let decision_path = args
        .tune_use
        .as_deref()
        .ok_or_else(|| "internal tune replay request is missing --tune-use".to_string())?;
    let decision_bytes = fs::read(decision_path)
        .map_err(|error| format!("read tuning decision {decision_path}: {error}"))?;
    let decision = decode_tune_decision(&decision_bytes).map_err(|error| error.to_string())?;
    let required = decision
        .replay_requirements()
        .map_err(|error| error.to_string())?;

    let protected = absolutize(Path::new(decision_path))?;
    let requested = require_out(args, "build")?;
    let requested_paths = NativeArtifactPaths::new(
        NativePlatform::host(),
        match args.kind {
            Some(ArtifactKind::Executable) => NativeArtifactKind::Executable,
            Some(ArtifactKind::Dynamic) => NativeArtifactKind::Dynamic,
            _ => return Err("Tune replay requires executable or dynamic output".to_string()),
        },
        &absolutize(Path::new(requested))?,
    );
    if [
        Some(&requested_paths.primary),
        requested_paths.header.as_ref(),
        requested_paths.import_library.as_ref(),
    ]
    .into_iter()
    .flatten()
    .any(|path| path == &protected)
    {
        return Err("Tune replay output must not overwrite its decision input".to_string());
    }

    let product = compile_verified_native_product(args)?;
    let actual_kind = match product.artifact_kind {
        NativeArtifactKind::Executable => TuneArtifactKind::Executable,
        NativeArtifactKind::Dynamic => TuneArtifactKind::Dynamic,
        NativeArtifactKind::Static | NativeArtifactKind::Object => {
            return Err("Tune replay produced an unsupported artifact kind".to_string());
        }
    };
    let pre_tune_kir_digest =
        calckernel::tuning_pre_kir_digest(&product.state).map_err(|error| error.to_string())?;
    if required.compiler_version != env!("CARGO_PKG_VERSION")
        || required.compiler_source != compiler_source_identity()
        || required.llvm_bridge != decode_hex_digest(env!("CKC_LLVM_MANIFEST_SHA256"))?
        || required.source_digest != product.source_digest
        || required.semantic_contract_digest != product.semantic_contract_digest
        || required.pre_tune_kir_digest != pre_tune_kir_digest
        || required.compilation_mode_digest != product.compilation_mode_digest
        || required.output_kind != actual_kind as u8
        || required.target_triple != product.target_triple
        || required.target_cpu != product.target_cpu
        || required.target_features != product.target_features
        || required.target_profile != product.target_profile
        || required.profile_digest != product.profile_digest
    {
        return Err(
            "tuning decision is stale for this exact source, compiler, mode, or target".to_string(),
        );
    }

    let space = enumerate_tuning_space(&product.state).map_err(|error| error.to_string())?;
    let frontier = run_deterministic_search(&product.state, &space, required.budget)
        .map_err(|error| error.to_string())?;
    if canonical_frontier_digest(&space, &frontier) != required.frontier_digest {
        return Err(
            "tuning decision frontier does not match current compiler analysis".to_string(),
        );
    }
    let baseline = TuningPlan::baseline();
    let selected = if required.selected_plan_digest == baseline.digest {
        baseline
    } else {
        frontier
            .compile_selection
            .iter()
            .find(|plan| plan.digest == required.selected_plan_digest)
            .cloned()
            .ok_or_else(|| {
                "tuning decision selected plan is absent from the current frontier".to_string()
            })?
    };
    let replayed = calckernel::apply_tuning_plan(&product.state, &space, &selected)
        .map_err(|error| error.to_string())?;
    let post_state_digest =
        calckernel::tuning_kir_state_digest(&replayed).map_err(|error| error.to_string())?;
    let recorded_pre = selected
        .choices
        .first()
        .map_or(space.pre_state_digest, |choice| choice.pre_state_digest);
    let recorded_post = selected
        .choices
        .last()
        .map_or(post_state_digest, |choice| choice.post_state_digest);
    if recorded_pre != required.selected_pre_state_digest
        || recorded_post != required.selected_post_state_digest
        || post_state_digest != required.selected_post_state_digest
    {
        return Err("tuning decision selected-plan state identity mismatch".to_string());
    }

    let replay_build = compile_replayed_native_build(&product, &replayed)?;
    let trial = compile_tune_trial(
        &product.state,
        &space,
        &selected,
        TuneTrialBuildRequest::from_verified_native_build(&replay_build)?,
    )?;
    if trial.identity().object_graph_digest != required.object_graph_digest
        || trial.identity().link_recipe_digest != required.link_recipe_digest
    {
        return Err("tuning replay object graph or link recipe mismatch".to_string());
    }
    let mut actual_roles = trial
        .identity()
        .roles
        .iter()
        .map(|role| role.role as u8)
        .collect::<Vec<_>>();
    let mut expected_roles = required
        .outputs
        .iter()
        .map(|output| output.role)
        .collect::<Vec<_>>();
    actual_roles.sort_unstable();
    expected_roles.sort_unstable();
    if actual_roles != expected_roles {
        return Err("tuning replay output-role set mismatch".to_string());
    }

    publish_verified_native_build(&product.paths, product.artifact_kind, &replay_build)?;
    println!(
        "OK: built native {} with verified tuning replay",
        match actual_kind {
            TuneArtifactKind::Executable => "executable",
            TuneArtifactKind::Dynamic => "dynamic library",
        }
    );
    println!("{}", product.paths.primary.display());
    if let Some(path) = &product.paths.header {
        println!("{}", path.display());
    }
    if let Some(path) = &product.paths.import_library {
        println!("{}", path.display());
    }
    Ok(())
}

#[cfg(feature = "native-toolchain")]
fn run_build(args: &ParsedArgs) -> Result<(), String> {
    let input = require_input(args, "tune build")?;
    let out = require_out(args, "tune build")?;
    let config = args
        .tune_config
        .as_deref()
        .ok_or_else(|| "Usage error for 'tune build': missing --config.".to_string())?;
    let budget = args.tune_budget.unwrap_or(TuneBudget::Standard);
    let config_bytes =
        fs::read(config).map_err(|error| format!("read tuning manifest {config}: {error}"))?;
    let manifest = TuneManifest::parse(&config_bytes, Path::new(config))
        .map_err(|error| format!("invalid tuning manifest: {error}"))?;
    let workload =
        capture_workload(&manifest).map_err(|error| format!("capture tuning workload: {error}"))?;

    let (artifact_kind, tune_kind) = match args.kind {
        Some(ArtifactKind::Executable) => {
            (NativeArtifactKind::Executable, TuneArtifactKind::Executable)
        }
        Some(ArtifactKind::Dynamic) => (NativeArtifactKind::Dynamic, TuneArtifactKind::Dynamic),
        _ => return Err("'tune build' requires executable or dynamic output".to_string()),
    };
    let paths = NativeArtifactPaths::new(
        NativePlatform::host(),
        artifact_kind,
        &absolutize(Path::new(out))?,
    );
    let decision_path = args.tune_out.as_deref().map_or_else(
        || append_suffix(&paths.primary, ".cktune"),
        |path| absolutize(Path::new(path)).unwrap_or_else(|_| PathBuf::from(path)),
    );
    let mut protected = vec![
        absolutize(Path::new(input))?,
        absolutize(Path::new(config))?,
    ];
    if let Some(profile) = &args.pgo_use {
        protected.push(absolutize(Path::new(profile))?);
    }
    protected.extend(manifest.protected_paths());
    let output_set = calckernel::TuneOutputSet::resolve(&paths, &decision_path, &protected)
        .map_err(|error| error.to_string())?;

    let source_bytes = fs::read(input).map_err(|error| format!("read source {input}: {error}"))?;
    let source_digest: [u8; 32] = Sha256::digest(&source_bytes).into();
    let early = early_identity(args, tune_kind, source_digest, workload.manifest_digest())?;
    let cache = if args.no_tune_cache {
        None
    } else {
        TuneCache::open_default()?
    };
    let decision_key = cache.as_ref().map(|cache| {
        cache.derive_key(
            TuneCacheDomain::Decision,
            &[
                &early.key_digest,
                output_identity_material(&paths).as_slice(),
            ],
        )
    });
    if let (Some(cache), Some(key)) = (&cache, decision_key)
        && let Some(hit) = cache.read(TuneCacheDomain::Decision, key)?
    {
        let (decision_bytes, artifacts) = decode_completed_package(hit.payload())?;
        let decision = decode_tune_decision(&decision_bytes).map_err(|error| error.to_string())?;
        let requirements = decision
            .replay_requirements()
            .map_err(|error| error.to_string())?;
        let baseline = TuningPlan::baseline();
        let interrupted_baseline = requirements.selected_plan_digest == baseline.digest
            && decision
                .has_candidate_timeout()
                .map_err(|error| error.to_string())?;
        if !interrupted_baseline {
            verify_warm_identity(&decision, &early, &paths, &artifacts)?;
            let mut publication = PublicationSet::acquire_and_recover(output_set.clone())
                .map_err(|error| error.to_string())?;
            publication
                .publish_verified(&decision, artifacts)
                .map_err(|error| error.to_string())?;
            print_tune_outputs(&paths, &decision_path, true);
            return Ok(());
        }
    }

    let product = compile_verified_native_product(args)?;
    verify_product_matches_early(&product, &early)?;
    let space = enumerate_tuning_space(&product.state).map_err(|error| error.to_string())?;
    let frontier = run_deterministic_search(&product.state, &space, budget)
        .map_err(|error| error.to_string())?;
    let baseline_plan = TuningPlan::baseline();
    let baseline_request = TuneTrialBuildRequest::from_verified_native_build(&product.build)?;
    let baseline = compile_tune_trial(&product.state, &space, &baseline_plan, baseline_request)?;
    let decision_identity = TuneDecisionIdentity {
        compiler_source: compiler_source_identity(),
        llvm_bridge: decode_hex_digest(env!("CKC_LLVM_MANIFEST_SHA256"))?,
        source_digest: product.source_digest,
        semantic_contract_digest: product.semantic_contract_digest,
        pre_tune_kir_digest: calckernel::tuning_pre_kir_digest(&product.state)
            .map_err(|error| error.to_string())?,
        compilation_mode_digest: product.compilation_mode_digest,
        output_kind: tune_kind,
        target_triple: product.target_triple.clone(),
        target_cpu: product.target_cpu.clone(),
        target_features: product.target_features.clone(),
        target_profile: product.target_profile.clone(),
        profile_digest: product.profile_digest,
    };

    let mut trials = Vec::with_capacity(frontier.compile_selection.len());
    let mut trial_builds = BTreeMap::new();
    let mut compile_origins = BTreeMap::new();
    for plan in &frontier.compile_selection {
        let cached = if let Some(cache) = cache.as_ref() {
            let key =
                cache.derive_key(TuneCacheDomain::Compile, &[&early.key_digest, &plan.digest]);
            cache.read(TuneCacheDomain::Compile, key)?.and_then(|hit| {
                decode_trial_build(hit.payload(), tune_kind)
                    .ok()
                    .map(|request| (request, *hit.receipt()))
            })
        } else {
            None
        };
        if let Some((request, receipt)) = cached {
            trials.push(compile_tune_trial(&product.state, &space, plan, request)?);
            let _ = receipt;
            compile_origins.insert(plan.digest, true);
            continue;
        }

        let replayed = calckernel::apply_tuning_plan(&product.state, &space, plan)
            .map_err(|error| error.to_string())?;
        let candidate_build = compile_replayed_native_build(&product, &replayed)?;
        let request = TuneTrialBuildRequest::from_verified_native_build(&candidate_build)?;
        let trial = compile_tune_trial(&product.state, &space, plan, request)?;
        let payload = encode_trial_build(&candidate_build)?;
        let origin = cache_origin_for(
            cache.as_ref(),
            TuneCacheDomain::Compile,
            &[&early.key_digest, &plan.digest],
            &payload,
        )?;
        if origin.reused {
            return Err("compile-cache miss was published as a reused origin".to_string());
        }
        compile_origins.insert(plan.digest, false);
        trials.push(trial);
        trial_builds.insert(plan.digest, candidate_build);
    }
    trials.sort_by_key(NonPublishableTuneTrial::plan_digest);
    verify_tune_trials_with_source(&product.state, &space, budget, &trials)?;
    for trial in &trials {
        let plan = frontier
            .compile_selection
            .iter()
            .find(|plan| plan.digest == trial.plan_digest())
            .ok_or_else(|| "trial is absent from compile selection".to_string())?;
        let replayed = calckernel::apply_tuning_plan(&product.state, &space, plan)
            .map_err(|error| error.to_string())?;
        let rebuilt = compile_replayed_native_build(&product, &replayed)?;
        let independently_rebuilt = compile_tune_trial(
            &product.state,
            &space,
            plan,
            TuneTrialBuildRequest::from_verified_native_build(&rebuilt)?,
        )?;
        if independently_rebuilt.identity() != trial.identity()
            || independently_rebuilt.primary_size() != trial.primary_size()
        {
            return Err("isolated tuning trial rebuild identity mismatch".to_string());
        }
        trial_builds.insert(plan.digest, rebuilt);
    }
    let finalists = select_size_valid_finalists(&baseline, &trials, budget)?;

    let baseline_artifact_package = encode_trial_build(&product.build)?;
    let baseline_origin = cache_origin_for(
        cache.as_ref(),
        TuneCacheDomain::Compile,
        &[&early.key_digest, &baseline_plan.digest],
        &baseline_artifact_package,
    )?;
    let runner = TuneRunner::new();
    let started = Instant::now();
    let calibrations = calibrate_cases(
        &runner,
        &workload,
        &baseline,
        remaining_wall(started, budget)?,
    )
    .map_err(|error| format!("tuning calibration failed: {error}"))?;
    let calibration_pairs = calibrations
        .iter()
        .map(|record| (record.case_id.as_str(), record.iterations))
        .collect::<Vec<_>>();
    let mut eligible = finalists.eligible;
    eligible.sort_by_key(NonPublishableTuneTrial::plan_digest);
    let baseline_digest = baseline.plan_digest();
    let mut channels = vec![MeasurementChannel::baseline(
        baseline_digest,
        baseline.primary_size(),
    )];
    channels.extend(eligible.iter().map(|trial| {
        let plan = frontier
            .compile_selection
            .iter()
            .find(|plan| plan.digest == trial.plan_digest())
            .expect("eligible plan");
        MeasurementChannel::candidate(
            plan.digest,
            trial.primary_size(),
            u32::try_from(plan.choices.len()).unwrap_or(u32::MAX),
        )
    }));
    let session_digest = derive_tune_session_digest(
        &decision_identity,
        budget,
        &workload,
        &space,
        &frontier,
        &baseline,
    )?;
    let mut scheduler = MeasurementScheduler::new(
        session_digest,
        channels,
        workload.case_identities().to_vec(),
        &calibration_pairs,
    )
    .map_err(|error| error.to_string())?;
    let mut invoke = |_: &calckernel::MeasurementCoordinate,
                      case: &TuneCase,
                      channel: &MeasurementChannel,
                      iterations: u64| {
        let trial = trial_for(channel.plan_digest, baseline_digest, &baseline, &eligible)
            .expect("frozen measurement channel");
        runner.invoke(
            &workload,
            trial,
            &TuneInvocation::new(
                case,
                iterations,
                remaining_wall(started, budget).unwrap_or(0),
            )
            .candidate(!channel.is_baseline),
        )
    };
    let smoke = scheduler
        .run_smoke(&mut invoke)
        .map_err(|error| format!("tuning smoke failed: {error}"))?;
    let search_run = scheduler
        .run_search(&mut invoke)
        .map_err(|error| format!("tuning search measurement failed: {error}"))?;
    let timed_out = timeout_plans(&smoke, &search_run);
    let ranks = eligible
        .iter()
        .filter(|trial| !timed_out.contains(&trial.plan_digest()))
        .map(|trial| {
            let plan = frontier
                .compile_selection
                .iter()
                .find(|plan| plan.digest == trial.plan_digest())
                .expect("finalist plan");
            calckernel::CandidateRank {
                plan_digest: plan.digest,
                primary_artifact_bytes: trial.primary_size(),
                choice_count: u32::try_from(plan.choices.len()).unwrap_or(u32::MAX),
            }
        })
        .collect::<Vec<_>>();
    let entrants = derive_search_entrants(
        baseline_digest,
        &ranks,
        workload.case_identities(),
        &search_run.streams,
        budget.contract().validation_entrant_limit,
    )
    .map_err(|error| error.to_string())?;
    let mut entrant_digests = entrants
        .iter()
        .map(|entrant| entrant.plan_digest)
        .collect::<Vec<_>>();
    entrant_digests.sort();
    let validation_one = scheduler
        .run_validation_round(1, &entrant_digests, &mut invoke)
        .map_err(|error| format!("tuning validation round one failed: {error}"))?;
    let validation_two = scheduler
        .run_validation_round(2, &entrant_digests, &mut invoke)
        .map_err(|error| format!("tuning validation round two failed: {error}"))?;
    if !validation_one.timeouts.is_empty() || !validation_two.timeouts.is_empty() {
        return Err("candidate timeout prevented complete validation".to_string());
    }
    let entrant_ranks = ranks
        .iter()
        .filter(|rank| entrant_digests.binary_search(&rank.plan_digest).is_ok())
        .copied()
        .collect::<Vec<_>>();
    let round_one = if entrant_ranks.is_empty() {
        empty_round(1)
    } else {
        derive_round_summary(
            1,
            baseline_digest,
            &entrant_ranks,
            workload.case_identities(),
            &validation_one.streams,
        )
        .map_err(|error| error.to_string())?
    };
    let round_two = if entrant_ranks.is_empty() {
        empty_round(2)
    } else {
        derive_round_summary(
            2,
            baseline_digest,
            &entrant_ranks,
            workload.case_identities(),
            &validation_two.streams,
        )
        .map_err(|error| error.to_string())?
    };
    let selection_entrants = entrant_digests
        .iter()
        .copied()
        .map(SelectionEntrant::active)
        .collect::<Vec<_>>();
    let selection = derive_selection(baseline_digest, &selection_entrants, &round_one, &round_two)
        .map_err(|error| error.to_string())?;

    let all_streams = search_run
        .streams
        .iter()
        .chain(validation_one.streams.iter())
        .chain(validation_two.streams.iter())
        .cloned()
        .collect::<Vec<_>>();
    let measurement_payload = measurement_cache_payload(
        &calibrations,
        &smoke,
        &search_run,
        &validation_one,
        &validation_two,
    );
    if let Some(cache) = cache.as_ref() {
        let key = cache.derive_key(
            TuneCacheDomain::Measurement,
            &[&early.key_digest, &session_digest],
        );
        cache.write(TuneCacheDomain::Measurement, key, &measurement_payload)?;
    }
    let eligible_set = eligible
        .iter()
        .map(NonPublishableTuneTrial::plan_digest)
        .collect::<BTreeSet<_>>();
    let size_rejected = finalists
        .size_rejected
        .iter()
        .map(NonPublishableTuneTrial::plan_digest)
        .collect::<BTreeSet<_>>();
    let entrant_set = entrant_digests.iter().copied().collect::<BTreeSet<_>>();
    let mut decision_candidates = Vec::new();
    for trial in &trials {
        let digest = trial.plan_digest();
        let plan = frontier
            .compile_selection
            .iter()
            .find(|plan| plan.digest == digest)
            .expect("compiled plan");
        let outcome = if size_rejected.contains(&digest) {
            CandidateOutcome::SizeRejected
        } else if !eligible_set.contains(&digest) {
            CandidateOutcome::CompiledUnmeasured
        } else if timed_out.contains(&digest) {
            CandidateOutcome::TimedOut
        } else if !entrant_set.contains(&digest) {
            CandidateOutcome::SearchNonwinner
        } else {
            *selection.outcomes.get(&digest).expect("validation outcome")
        };
        let streams = all_streams
            .iter()
            .filter(|stream| stream.plan_digest == digest)
            .cloned()
            .collect::<Vec<_>>();
        let timeout = smoke
            .timeouts
            .iter()
            .chain(search_run.timeouts.iter())
            .find(|timeout| timeout.plan_digest == digest)
            .cloned();
        decision_candidates.push(TuneDecisionCandidate {
            plan,
            trial,
            outcome,
            streams,
            timeout,
            compile_reused: compile_origins[&digest],
        });
    }
    let baseline_streams = all_streams
        .iter()
        .filter(|stream| stream.plan_digest == baseline_digest)
        .cloned()
        .collect::<Vec<_>>();
    let selected_build = if selection.selected_plan_digest == baseline_digest {
        &product.build
    } else {
        trial_builds
            .get(&selection.selected_plan_digest)
            .ok_or_else(|| "selected tuning build disappeared".to_string())?
    };
    let outputs = decision_outputs(&paths, selected_build)?;
    let build_input = TuneDecisionBuildInput {
        identity: decision_identity,
        budget,
        workload: &workload,
        calibrations: &calibrations,
        space: &space,
        frontier: &frontier,
        baseline: &baseline,
        baseline_streams,
        baseline_compile_reused: baseline_origin.reused,
        candidates: decision_candidates,
        round_one: &round_one,
        round_two: &round_two,
        selection: &selection,
        measurement_reused: false,
        measurement_cache_salt_digest: cache.as_ref().map_or_else(
            || hash_parts(b"CK-TUNE-NO-CACHE-SALT\0", &[&early.key_digest]),
            |cache| *cache.salt_digest(),
        ),
        outputs,
    };
    let encoded =
        encode_completed_tune_decision(&build_input).map_err(|error| error.to_string())?;
    let decision = assemble_decision(encoded, |candidate| {
        verify_generated_decision(candidate, &build_input, &product)
    })
    .map_err(|error| error.to_string())?;
    let artifacts = TunePublishArtifacts {
        primary: selected_build.primary().to_vec(),
        header: selected_build.header().map(<[u8]>::to_vec),
        import_library: selected_build.import_library().map(<[u8]>::to_vec),
    };
    let completed = encode_completed_package(decision.as_bytes(), &artifacts)?;
    let mut publication =
        PublicationSet::acquire_and_recover(output_set).map_err(|error| error.to_string())?;
    publication
        .publish_verified(&decision, artifacts)
        .map_err(|error| error.to_string())?;
    let cacheable_completed =
        selection.selected_plan_digest != baseline_digest || timed_out.is_empty();
    if cacheable_completed && let (Some(cache), Some(key)) = (&cache, decision_key) {
        cache.write(TuneCacheDomain::Decision, key, &completed)?;
    }
    print_tune_outputs(&paths, &decision_path, false);
    Ok(())
}

#[cfg(feature = "native-toolchain")]
struct EarlyIdentity {
    key_digest: [u8; 32],
    compiler_source: [u8; 32],
    source_digest: [u8; 32],
    semantic_contract_digest: [u8; 32],
    compilation_mode_digest: [u8; 32],
    output_kind: TuneArtifactKind,
    target_triple: String,
    target_cpu: String,
    target_features: Vec<String>,
    target_profile: String,
    profile_digest: Option<[u8; 32]>,
}

#[cfg(feature = "native-toolchain")]
fn early_identity(
    args: &ParsedArgs,
    output_kind: TuneArtifactKind,
    source_digest: [u8; 32],
    manifest_digest: [u8; 32],
) -> Result<EarlyIdentity, String> {
    let target =
        NativeTarget::host_with_cpu(NativeCpu::Native).map_err(|error| error.to_string())?;
    let target_triple = target.triple().map_err(|error| error.to_string())?;
    if args
        .target
        .as_deref()
        .is_some_and(|requested| requested != target_triple)
    {
        return Err("offline tuning target must equal the exact host triple".to_string());
    }
    let target_cpu = target.cpu().map_err(|error| error.to_string())?;
    let mut target_features = target
        .features()
        .map_err(|error| error.to_string())?
        .split(',')
        .filter(|feature| !feature.is_empty())
        .map(str::to_string)
        .collect::<Vec<_>>();
    target_features.sort();
    target_features.dedup();
    let consumer = match output_kind {
        TuneArtifactKind::Executable => calckernel::KirConsumer::NativeExecutable,
        TuneArtifactKind::Dynamic => calckernel::KirConsumer::NativeLibrary,
    };
    let target_profile = target
        .kir_profile(consumer)
        .map_err(|error| error.to_string())?
        .digest_hex();
    let overflow = parse_overflow_mode(args)?;
    let bounds = parse_bounds_mode(args)?;
    let semantic_contract_digest = hash_parts(
        b"CK-TUNE-SEMANTIC-CONTRACT\0",
        &[
            &[match overflow {
                calckernel::OverflowMode::Unchecked => 0,
                calckernel::OverflowMode::Checked => 1,
            }],
            &[match bounds {
                calckernel::BoundsMode::Unchecked => 0,
                calckernel::BoundsMode::Checked => 1,
            }],
            b"strict-f64",
        ],
    );
    let compilation_mode_digest = hash_parts(
        b"CK-TUNE-COMPILATION-MODE\0",
        &[
            &[parse_opt_level(args)?],
            &[match overflow {
                calckernel::OverflowMode::Unchecked => 0,
                calckernel::OverflowMode::Checked => 1,
            }],
            &[match bounds {
                calckernel::BoundsMode::Unchecked => 0,
                calckernel::BoundsMode::Checked => 1,
            }],
            &[output_kind as u8],
            b"cpu-native",
        ],
    );
    let profile_digest = args
        .pgo_use
        .as_deref()
        .map(fs::read)
        .transpose()
        .map_err(|error| format!("read profile: {error}"))?
        .map(|bytes| Sha256::digest(bytes).into());
    let compiler_source = compiler_source_identity();
    let features_bytes = target_features.join(",");
    let profile_material = profile_digest.unwrap_or([0; 32]);
    let key_digest = hash_parts(
        b"CK-TUNE-COMPLETED-INPUT\0",
        &[
            env!("CARGO_PKG_VERSION").as_bytes(),
            &compiler_source,
            &source_digest,
            &manifest_digest,
            &semantic_contract_digest,
            &compilation_mode_digest,
            target_triple.as_bytes(),
            target_cpu.as_bytes(),
            features_bytes.as_bytes(),
            target_profile.as_bytes(),
            &profile_material,
        ],
    );
    Ok(EarlyIdentity {
        key_digest,
        compiler_source,
        source_digest,
        semantic_contract_digest,
        compilation_mode_digest,
        output_kind,
        target_triple,
        target_cpu,
        target_features,
        target_profile,
        profile_digest,
    })
}

#[cfg(feature = "native-toolchain")]
fn verify_product_matches_early(
    product: &NativeBuildProduct,
    early: &EarlyIdentity,
) -> Result<(), String> {
    let actual_pre =
        calckernel::tuning_pre_kir_digest(&product.state).map_err(|error| error.to_string())?;
    if product.source_digest != early.source_digest
        || product.semantic_contract_digest != early.semantic_contract_digest
        || product.compilation_mode_digest != early.compilation_mode_digest
        || product.target_triple != early.target_triple
        || product.target_cpu != early.target_cpu
        || product.target_features != early.target_features
        || product.target_profile != early.target_profile
        || product.profile_digest != early.profile_digest
        || actual_pre == [0; 32]
    {
        return Err("source-aware baseline identity changed during compilation".to_string());
    }
    Ok(())
}

#[cfg(feature = "native-toolchain")]
fn trial_for<'a>(
    digest: [u8; 32],
    baseline_digest: [u8; 32],
    baseline: &'a NonPublishableTuneTrial,
    trials: &'a [NonPublishableTuneTrial],
) -> Option<&'a NonPublishableTuneTrial> {
    if digest == baseline_digest {
        Some(baseline)
    } else {
        trials.iter().find(|trial| trial.plan_digest() == digest)
    }
}

#[cfg(feature = "native-toolchain")]
fn timeout_plans(runs_a: &MeasurementRun, runs_b: &MeasurementRun) -> BTreeSet<[u8; 32]> {
    runs_a
        .timeouts
        .iter()
        .chain(runs_b.timeouts.iter())
        .map(|timeout| timeout.plan_digest)
        .collect()
}

#[cfg(feature = "native-toolchain")]
fn empty_round(round: u8) -> RoundSummary {
    RoundSummary {
        round,
        plans: Vec::new(),
        ranked_plan_digests: Vec::new(),
    }
}

#[cfg(feature = "native-toolchain")]
fn remaining_wall(started: Instant, budget: TuneBudget) -> Result<u64, String> {
    budget
        .contract()
        .wall_clock_ms
        .checked_sub(u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX))
        .ok_or_else(|| "offline tuning wall-clock budget exhausted".to_string())
}

#[cfg(feature = "native-toolchain")]
fn cache_origin_for(
    cache: Option<&TuneCache>,
    domain: TuneCacheDomain,
    materials: &[&[u8]],
    payload: &[u8],
) -> Result<TuneRecordedCacheOrigin, String> {
    if let Some(cache) = cache {
        let key = cache.derive_key(domain, materials);
        if let Some(hit) = cache.read(domain, key)?
            && hit.payload() == payload
        {
            return Ok(TuneRecordedCacheOrigin {
                reused: true,
                key_digest: *hit.receipt().key_digest(),
                entry_digest: *hit.receipt().entry_digest(),
            });
        }
        let receipt = cache.write(domain, key, payload)?;
        Ok(TuneRecordedCacheOrigin {
            reused: false,
            key_digest: *receipt.key_digest(),
            entry_digest: *receipt.entry_digest(),
        })
    } else {
        Ok(TuneRecordedCacheOrigin {
            reused: false,
            key_digest: hash_parts(domain_name(domain), materials),
            entry_digest: hash_parts(b"CK-TUNE-CACHE-ENTRY\0", &[payload]),
        })
    }
}

#[cfg(feature = "native-toolchain")]
fn domain_name(domain: TuneCacheDomain) -> &'static [u8] {
    match domain {
        TuneCacheDomain::Compile => b"CK-TUNE-COMPILE-KEY\0",
        TuneCacheDomain::Measurement => b"CK-TUNE-MEASUREMENT-KEY\0",
        TuneCacheDomain::Decision => b"CK-TUNE-COMPLETED-DECISION-KEY\0",
    }
}

#[cfg(feature = "native-toolchain")]
fn measurement_cache_payload(
    calibrations: &[calckernel::CalibrationRecord],
    smoke: &MeasurementRun,
    search: &MeasurementRun,
    one: &MeasurementRun,
    two: &MeasurementRun,
) -> Vec<u8> {
    format!("{calibrations:?}\n{smoke:?}\n{search:?}\n{one:?}\n{two:?}").into_bytes()
}

#[cfg(feature = "native-toolchain")]
fn decision_outputs(
    paths: &NativeArtifactPaths,
    build: &calckernel::VerifiedNativeBuild,
) -> Result<Vec<TuneDecisionOutput>, String> {
    let mut outputs = vec![output_record(
        TuneArtifactRole::Primary,
        &paths.primary,
        build.primary(),
    )?];
    if let (Some(path), Some(bytes)) = (&paths.header, build.header()) {
        outputs.push(output_record(TuneArtifactRole::Header, path, bytes)?);
    }
    if let (Some(path), Some(bytes)) = (&paths.import_library, build.import_library()) {
        outputs.push(output_record(TuneArtifactRole::ImportLibrary, path, bytes)?);
    }
    Ok(outputs)
}

#[cfg(feature = "native-toolchain")]
fn output_record(
    role: TuneArtifactRole,
    path: &Path,
    bytes: &[u8],
) -> Result<TuneDecisionOutput, String> {
    let logical_basename = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| "tuning output basename is not UTF-8".to_string())?
        .to_string();
    Ok(TuneDecisionOutput {
        role,
        logical_basename,
        content_digest: Sha256::digest(bytes).into(),
        content_bytes: u64::try_from(bytes.len()).map_err(|_| "output size overflow")?,
    })
}

#[cfg(feature = "native-toolchain")]
fn output_identity_material(paths: &NativeArtifactPaths) -> Vec<u8> {
    [
        Some(&paths.primary),
        paths.header.as_ref(),
        paths.import_library.as_ref(),
    ]
    .into_iter()
    .flatten()
    .filter_map(|path| path.file_name().and_then(|name| name.to_str()))
    .collect::<Vec<_>>()
    .join("\0")
    .into_bytes()
}

#[cfg(feature = "native-toolchain")]
fn encode_artifacts(artifacts: &TunePublishArtifacts) -> Result<Vec<u8>, String> {
    let mut out = b"CKTART01".to_vec();
    for bytes in [
        Some(artifacts.primary.as_slice()),
        artifacts.header.as_deref(),
        artifacts.import_library.as_deref(),
    ] {
        match bytes {
            Some(bytes) => {
                out.push(1);
                out.extend_from_slice(
                    &u64::try_from(bytes.len())
                        .map_err(|_| "artifact package overflow")?
                        .to_be_bytes(),
                );
                out.extend_from_slice(bytes);
            }
            None => out.extend_from_slice(&[0]),
        }
    }
    Ok(out)
}

#[cfg(feature = "native-toolchain")]
fn decode_artifacts(bytes: &[u8]) -> Result<TunePublishArtifacts, String> {
    if bytes.get(..8) != Some(b"CKTART01") {
        return Err("invalid completed tuning artifact package".to_string());
    }
    let mut offset = 8;
    let mut fields = Vec::new();
    for _ in 0..3 {
        let present = *bytes.get(offset).ok_or("truncated artifact package")?;
        offset += 1;
        if present == 0 {
            fields.push(None);
            continue;
        }
        if present != 1 {
            return Err("invalid artifact package presence".to_string());
        }
        let length = u64::from_be_bytes(
            bytes
                .get(offset..offset + 8)
                .ok_or("truncated artifact package length")?
                .try_into()
                .map_err(|_| "artifact package length")?,
        );
        offset += 8;
        let end = offset
            .checked_add(usize::try_from(length).map_err(|_| "artifact package overflow")?)
            .ok_or("artifact package overflow")?;
        fields.push(Some(
            bytes
                .get(offset..end)
                .ok_or("truncated artifact package bytes")?
                .to_vec(),
        ));
        offset = end;
    }
    if offset != bytes.len() || fields[0].as_ref().is_none_or(Vec::is_empty) {
        return Err("invalid artifact package tail".to_string());
    }
    Ok(TunePublishArtifacts {
        primary: fields.remove(0).expect("primary"),
        header: fields.remove(0),
        import_library: fields.remove(0),
    })
}

#[cfg(feature = "native-toolchain")]
fn encode_trial_build(build: &calckernel::VerifiedNativeBuild) -> Result<Vec<u8>, String> {
    let kind = match build.kind() {
        NativeArtifactKind::Executable => TuneArtifactKind::Executable,
        NativeArtifactKind::Dynamic => TuneArtifactKind::Dynamic,
        NativeArtifactKind::Static | NativeArtifactKind::Object => {
            return Err("offline tuning cannot cache this artifact kind".to_string());
        }
    };
    let artifacts = encode_artifacts(&TunePublishArtifacts {
        primary: build.primary().to_vec(),
        header: build.header().map(<[u8]>::to_vec),
        import_library: build.import_library().map(<[u8]>::to_vec),
    })?;
    let mut out = b"CKTBLD01".to_vec();
    out.push(kind as u8);
    append_bounded_blob(&mut out, &artifacts)?;
    out.extend_from_slice(
        &u32::try_from(build.object_graph().len())
            .map_err(|_| "trial-build object count overflow")?
            .to_be_bytes(),
    );
    for (name, bytes) in build.object_graph() {
        append_bounded_blob(&mut out, name.as_bytes())?;
        append_bounded_blob(&mut out, bytes)?;
    }
    out.extend_from_slice(
        &u32::try_from(build.link_recipe().len())
            .map_err(|_| "trial-build recipe count overflow")?
            .to_be_bytes(),
    );
    for item in build.link_recipe() {
        append_bounded_blob(&mut out, item.as_bytes())?;
    }
    Ok(out)
}

#[cfg(feature = "native-toolchain")]
fn decode_trial_build(
    bytes: &[u8],
    expected_kind: TuneArtifactKind,
) -> Result<TuneTrialBuildRequest, String> {
    let mut input = BoundedInput::new(bytes);
    if input.take(8)? != b"CKTBLD01" {
        return Err("invalid cached trial-build magic".to_string());
    }
    let kind = match input.take_u8()? {
        1 => TuneArtifactKind::Executable,
        2 => TuneArtifactKind::Dynamic,
        _ => return Err("invalid cached trial-build kind".to_string()),
    };
    if kind != expected_kind {
        return Err("cached trial-build kind mismatch".to_string());
    }
    let artifacts = decode_artifacts(input.take_blob(1024 * 1024 * 1024)?)?;
    let object_count = input.take_u32()?;
    if object_count == 0 || object_count > 64 {
        return Err("cached trial-build object count exceeds bounds".to_string());
    }
    let mut object_graph = Vec::with_capacity(
        usize::try_from(object_count).map_err(|_| "cached trial-build object count overflow")?,
    );
    for _ in 0..object_count {
        let name = String::from_utf8(input.take_blob(255)?.to_vec())
            .map_err(|_| "cached trial-build object name is not UTF-8".to_string())?;
        let object = input.take_blob(256 * 1024 * 1024)?.to_vec();
        object_graph.push((name, object));
    }
    let recipe_count = input.take_u32()?;
    if recipe_count > 128 {
        return Err("cached trial-build recipe count exceeds bounds".to_string());
    }
    let mut link_recipe = Vec::with_capacity(
        usize::try_from(recipe_count).map_err(|_| "cached trial-build recipe count overflow")?,
    );
    for _ in 0..recipe_count {
        link_recipe.push(
            String::from_utf8(input.take_blob(4_096)?.to_vec())
                .map_err(|_| "cached trial-build recipe is not UTF-8".to_string())?,
        );
    }
    if !input.is_empty() {
        return Err("cached trial-build has trailing bytes".to_string());
    }
    Ok(TuneTrialBuildRequest::new(
        kind,
        artifacts.primary,
        artifacts.header,
        artifacts.import_library,
        object_graph,
        link_recipe,
    ))
}

#[cfg(feature = "native-toolchain")]
fn append_bounded_blob(out: &mut Vec<u8>, bytes: &[u8]) -> Result<(), String> {
    out.extend_from_slice(
        &u64::try_from(bytes.len())
            .map_err(|_| "trial-build blob length overflow")?
            .to_be_bytes(),
    );
    out.extend_from_slice(bytes);
    Ok(())
}

#[cfg(feature = "native-toolchain")]
struct BoundedInput<'a> {
    bytes: &'a [u8],
    offset: usize,
}

#[cfg(feature = "native-toolchain")]
impl<'a> BoundedInput<'a> {
    const fn new(bytes: &'a [u8]) -> Self {
        Self { bytes, offset: 0 }
    }

    fn take(&mut self, length: usize) -> Result<&'a [u8], String> {
        let end = self
            .offset
            .checked_add(length)
            .ok_or_else(|| "cached trial-build length overflow".to_string())?;
        let value = self
            .bytes
            .get(self.offset..end)
            .ok_or_else(|| "truncated cached trial-build".to_string())?;
        self.offset = end;
        Ok(value)
    }

    fn take_u8(&mut self) -> Result<u8, String> {
        self.take(1).map(|bytes| bytes[0])
    }

    fn take_u32(&mut self) -> Result<u32, String> {
        self.take(4).and_then(|bytes| {
            bytes
                .try_into()
                .map(u32::from_be_bytes)
                .map_err(|_| "cached trial-build u32".to_string())
        })
    }

    fn take_blob(&mut self, maximum: usize) -> Result<&'a [u8], String> {
        let length = usize::try_from(u64::from_be_bytes(
            self.take(8)?
                .try_into()
                .map_err(|_| "cached trial-build blob length".to_string())?,
        ))
        .map_err(|_| "cached trial-build blob length overflow".to_string())?;
        if length > maximum {
            return Err("cached trial-build blob exceeds bounds".to_string());
        }
        self.take(length)
    }

    fn is_empty(&self) -> bool {
        self.offset == self.bytes.len()
    }
}

#[cfg(feature = "native-toolchain")]
fn encode_completed_package(
    decision: &[u8],
    artifacts: &TunePublishArtifacts,
) -> Result<Vec<u8>, String> {
    let artifact_bytes = encode_artifacts(artifacts)?;
    let mut out = b"CKTDONE1".to_vec();
    out.extend_from_slice(
        &u64::try_from(decision.len())
            .map_err(|_| "completed decision overflow")?
            .to_be_bytes(),
    );
    out.extend_from_slice(decision);
    out.extend_from_slice(&artifact_bytes);
    Ok(out)
}

#[cfg(feature = "native-toolchain")]
fn decode_completed_package(bytes: &[u8]) -> Result<(Vec<u8>, TunePublishArtifacts), String> {
    if bytes.get(..8) != Some(b"CKTDONE1") || bytes.len() < 16 {
        return Err("invalid completed tuning package".to_string());
    }
    let length = usize::try_from(u64::from_be_bytes(
        bytes[8..16]
            .try_into()
            .map_err(|_| "completed decision length")?,
    ))
    .map_err(|_| "completed decision overflow")?;
    let end = 16usize
        .checked_add(length)
        .ok_or("completed decision overflow")?;
    let decision = bytes
        .get(16..end)
        .ok_or("truncated completed decision")?
        .to_vec();
    let artifacts = decode_artifacts(bytes.get(end..).ok_or("truncated completed artifacts")?)?;
    Ok((decision, artifacts))
}

#[cfg(feature = "native-toolchain")]
fn verify_warm_identity(
    decision: &calckernel::TuneDecision,
    early: &EarlyIdentity,
    paths: &NativeArtifactPaths,
    artifacts: &TunePublishArtifacts,
) -> Result<(), String> {
    let required = decision
        .replay_requirements()
        .map_err(|error| error.to_string())?;
    if required.compiler_version != env!("CARGO_PKG_VERSION")
        || required.compiler_source != early.compiler_source
        || required.source_digest != early.source_digest
        || required.semantic_contract_digest != early.semantic_contract_digest
        || required.compilation_mode_digest != early.compilation_mode_digest
        || required.output_kind != early.output_kind as u8
        || required.target_triple != early.target_triple
        || required.target_cpu != early.target_cpu
        || required.target_features != early.target_features
        || required.target_profile != early.target_profile
        || required.profile_digest != early.profile_digest
    {
        return Err("completed tuning decision identity mismatch".to_string());
    }
    let actual = [
        Some((&paths.primary, artifacts.primary.as_slice())),
        paths.header.as_ref().zip(artifacts.header.as_deref()),
        paths
            .import_library
            .as_ref()
            .zip(artifacts.import_library.as_deref()),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>();
    if actual.len() != required.outputs.len() {
        return Err("completed tuning output set mismatch".to_string());
    }
    for ((path, bytes), recorded) in actual.into_iter().zip(&required.outputs) {
        if path.file_name().and_then(|name| name.to_str()) != Some(&recorded.logical_basename)
            || u64::try_from(bytes.len()).ok() != Some(recorded.content_bytes)
            || <[u8; 32]>::from(Sha256::digest(bytes)) != recorded.content_digest
        {
            return Err("completed tuning artifact identity mismatch".to_string());
        }
    }
    Ok(())
}

#[cfg(feature = "native-toolchain")]
fn verify_generated_decision(
    decision: &calckernel::TuneDecision,
    input: &TuneDecisionBuildInput<'_>,
    product: &NativeBuildProduct,
) -> Result<(), String> {
    let required = decision
        .replay_requirements()
        .map_err(|error| error.to_string())?;
    let pre_state =
        calckernel::tuning_pre_kir_digest(&product.state).map_err(|error| error.to_string())?;
    if required.compiler_version != env!("CARGO_PKG_VERSION")
        || required.compiler_source != input.identity.compiler_source
        || required.llvm_bridge != input.identity.llvm_bridge
        || required.source_digest != product.source_digest
        || required.semantic_contract_digest != product.semantic_contract_digest
        || required.pre_tune_kir_digest != pre_state
        || required.compilation_mode_digest != product.compilation_mode_digest
        || required.output_kind != input.identity.output_kind as u8
        || required.target_triple != product.target_triple
        || required.target_cpu != product.target_cpu
        || required.target_features != product.target_features
        || required.target_profile != product.target_profile
        || required.profile_digest != product.profile_digest
        || required.budget != input.budget
        || required.session_digest
            != derive_tune_session_digest(
                &input.identity,
                input.budget,
                input.workload,
                input.space,
                input.frontier,
                input.baseline,
            )?
        || required.frontier_digest != canonical_frontier_digest(input.space, input.frontier)
        || required.selected_plan_digest != input.selection.selected_plan_digest
    {
        return Err("generated tuning decision identity reconstruction mismatch".to_string());
    }

    let (plan, selected) = if required.selected_plan_digest == input.baseline.plan_digest() {
        (TuningPlan::baseline(), input.baseline)
    } else {
        let candidate = input
            .candidates
            .iter()
            .find(|candidate| candidate.plan.digest == required.selected_plan_digest)
            .ok_or_else(|| "generated decision selected an unknown candidate".to_string())?;
        (candidate.plan.clone(), candidate.trial)
    };
    let replayed = calckernel::apply_tuning_plan(&product.state, input.space, &plan)
        .map_err(|error| error.to_string())?;
    let expected_pre = plan
        .choices
        .first()
        .map_or(input.space.pre_state_digest, |choice| {
            choice.pre_state_digest
        });
    let replayed_post =
        calckernel::tuning_kir_state_digest(&replayed).map_err(|error| error.to_string())?;
    let expected_post = plan
        .choices
        .last()
        .map_or(replayed_post, |choice| choice.post_state_digest);
    if required.selected_pre_state_digest != expected_pre
        || required.selected_post_state_digest != expected_post
        || required.selected_post_state_digest != replayed_post
        || required.object_graph_digest != selected.identity().object_graph_digest
        || required.link_recipe_digest != selected.identity().link_recipe_digest
    {
        return Err("generated tuning decision replay reconstruction mismatch".to_string());
    }
    let rebuilt = compile_replayed_native_build(product, &replayed)?;
    let rebuilt_trial = compile_tune_trial(
        &product.state,
        input.space,
        &plan,
        TuneTrialBuildRequest::from_verified_native_build(&rebuilt)?,
    )?;
    if rebuilt_trial.identity() != selected.identity()
        || rebuilt_trial.primary_size() != selected.primary_size()
        || rebuilt_trial.plan_digest() != selected.plan_digest()
    {
        return Err(
            "generated tuning decision isolated selected-artifact rebuild mismatch".to_string(),
        );
    }

    let mut recorded = required.outputs;
    recorded.sort_by_key(|output| output.role);
    let mut actual = input.outputs.clone();
    actual.sort_by_key(|output| output.role);
    if recorded.len() != actual.len()
        || recorded.iter().zip(&actual).any(|(left, right)| {
            left.role != right.role as u8
                || left.logical_basename != right.logical_basename
                || left.content_digest != right.content_digest
                || left.content_bytes != right.content_bytes
        })
    {
        return Err("generated tuning decision output reconstruction mismatch".to_string());
    }
    Ok(())
}

#[cfg(feature = "native-toolchain")]
fn print_tune_outputs(paths: &NativeArtifactPaths, decision: &Path, warm: bool) {
    println!(
        "OK: tuned native artifact ({})",
        if warm {
            "warm exact reuse"
        } else {
            "fresh session"
        }
    );
    println!("{}", paths.primary.display());
    if let Some(path) = &paths.header {
        println!("{}", path.display());
    }
    if let Some(path) = &paths.import_library {
        println!("{}", path.display());
    }
    println!("{}", decision.display());
}

#[cfg(feature = "native-toolchain")]
fn append_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name: OsString = path.as_os_str().to_os_string();
    name.push(suffix);
    PathBuf::from(name)
}

#[cfg(feature = "native-toolchain")]
fn absolutize(path: &Path) -> Result<PathBuf, String> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| format!("resolve current directory: {error}"))
    }
}

#[cfg(feature = "native-toolchain")]
fn decode_hex_digest(value: &str) -> Result<[u8; 32], String> {
    if value.len() != 64 {
        return Err("digest is not 32-byte lowercase hex".to_string());
    }
    let mut digest = [0u8; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        let nibble = |value| match value {
            b'0'..=b'9' => Some(value - b'0'),
            b'a'..=b'f' => Some(value - b'a' + 10),
            _ => None,
        };
        digest[index] = (nibble(pair[0]).ok_or("invalid digest hex")? << 4)
            | nibble(pair[1]).ok_or("invalid digest hex")?;
    }
    Ok(digest)
}

#[cfg(feature = "native-toolchain")]
fn hash_parts(domain: &[u8], parts: &[&[u8]]) -> [u8; 32] {
    let mut digest = Sha256::new();
    digest.update(domain);
    for part in parts {
        digest.update(u64::try_from(part.len()).unwrap_or(u64::MAX).to_be_bytes());
        digest.update(part);
    }
    digest.finalize().into()
}
