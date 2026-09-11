use std::borrow::Borrow;

use super::*;

// Read the same semantic module before and after the retention representation
// changes, so the regression fails on a real deep copy rather than a missing API.
trait RetainedModule {
    fn retained_module(&self) -> &crate::KirModule;
}

impl RetainedModule for crate::KirModule {
    fn retained_module(&self) -> &crate::KirModule {
        self
    }
}

impl RetainedModule for Rc<KirVerifiedProgramState> {
    fn retained_module(&self) -> &crate::KirModule {
        self.module()
    }
}

fn shared_state() -> Rc<KirVerifiedProgramState> {
    let checked = crate::check(&crate::SourceFile::new(
        "snapshot-sharing.ck",
        "export fn answer(n: i32) -> i32 { return n; }",
    ));
    assert!(checked.diagnostics.is_empty());
    let mir = crate::lower_to_mir(&checked.checked_program).expect("MIR");
    let module = crate::build_kir_module(
        &mir,
        crate::KirBuildConfig {
            consumer: crate::KirConsumer::C,
            overflow_mode: crate::KirOverflowMode::Unchecked,
            bounds_mode: crate::KirBoundsMode::Unchecked,
            sanitizer_mode: crate::KirSanitizerMode::Disabled,
        },
    )
    .expect("KIR");
    Rc::new(crate::prepare_kir_pre_tune_state(module, None).expect("pre-tune"))
}

#[test]
fn analysis_cache_retains_the_existing_immutable_snapshot() {
    let state = shared_state();
    let expected = finish_selected_o3(&state).expect("independent phase");
    let mut cache = ReplayPrefixCache::default();
    let actual = cache.analyze(ReplayPhase::Finish, &state).expect("phase");
    assert_eq!(actual.as_ref(), &expected);
    let (input, _) = cache.analyses.values().next().expect("retained analysis");
    let retained: &KirVerifiedProgramState = input.borrow();
    assert!(
        std::ptr::eq(retained, state.as_ref()),
        "analysis retention deep-cloned an already shared immutable input"
    );
}

#[test]
fn identity_cache_retains_the_existing_immutable_module() {
    let state = shared_state();
    let expected = tuning_kir_state_identity(&state).expect("independent identity");
    let mut cache = ReplayPrefixCache::default();
    assert_eq!(cache.identity(&state).expect("cached identity"), expected);
    let (input, _, _) = cache.identities.values().next().expect("retained identity");
    assert!(
        std::ptr::eq(input.retained_module(), state.module()),
        "identity retention deep-cloned an already shared immutable module"
    );
}

#[test]
fn retained_analysis_stays_immutable_when_the_callers_handle_changes() {
    let mut state = shared_state();
    let original = state.as_ref().clone();
    let mut cache = ReplayPrefixCache::default();
    cache.analyze(ReplayPhase::Finish, &state).expect("phase");
    Rc::make_mut(&mut state).module_mut().functions[0].name = "changed".to_string();
    let (input, _) = cache.analyses.values().next().expect("retained analysis");
    let retained: &KirVerifiedProgramState = input.borrow();
    assert_eq!(retained, &original);
    assert_ne!(retained, state.as_ref());
    assert_eq!(
        cache
            .analyze(ReplayPhase::Finish, &state)
            .expect("new input")
            .as_ref(),
        &finish_selected_o3(&state).expect("independent new input")
    );
}

#[test]
fn each_search_retains_one_lazy_root_even_when_phase_entries_are_evicted() {
    let state = shared_state();
    let checked = CheckedTuningSpace::enumerate(&state).expect("space");
    let mut replay = checked.search_replay();
    assert!(replay.shared_state.get().is_none());
    replay.derive(&[]).expect("first root replay");
    let first_root = Rc::clone(replay.shared_state.get().expect("root retained"));
    let first_input = &replay
        .prefixes
        .analyses
        .values()
        .next()
        .expect("analysis")
        .0;
    assert!(Rc::ptr_eq(first_input, &first_root));
    replay.prefixes.analyses.clear();
    replay.prefixes.identities.clear();
    replay.derive(&[]).expect("new analysis after eviction");
    let second_input = &replay
        .prefixes
        .analyses
        .values()
        .next()
        .expect("analysis")
        .0;
    assert!(Rc::ptr_eq(second_input, &first_root));
    assert_eq!(second_input.as_ref(), state.as_ref());
}

#[test]
fn separate_searches_do_not_share_their_owned_root_snapshots() {
    let state = shared_state();
    let checked = CheckedTuningSpace::enumerate(&state).expect("space");
    let mut first = checked.search_replay();
    let mut second = checked.search_replay();
    first.derive(&[]).expect("first search");
    second.derive(&[]).expect("second search");
    let first_root = first.shared_state.get().expect("first root");
    let second_root = second.shared_state.get().expect("second root");
    assert!(!Rc::ptr_eq(first_root, second_root));
    assert_eq!(first_root, second_root);
}
