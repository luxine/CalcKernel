use calckernel::{
    DecisionAssemblyError, TuneDecisionError, assemble_decision, encode_tune_decision,
};

use super::support;

#[test]
fn session_assembly_requires_both_self_contained_and_source_aware_validation() {
    let bytes = support::baseline_decision();
    let decision = assemble_decision(bytes.clone(), |_| Ok(())).expect("validated assembly");
    assert_eq!(encode_tune_decision(&decision), bytes);

    let mut corrupt = support::baseline_decision();
    *corrupt.last_mut().expect("digest") ^= 1;
    assert_eq!(
        assemble_decision(corrupt, |_| Ok(())),
        Err(DecisionAssemblyError::Decision(
            TuneDecisionError::DigestMismatch
        ))
    );
    assert_eq!(
        assemble_decision(bytes, |_| Err("source replay mismatch".into())),
        Err(DecisionAssemblyError::SourceAware(
            "source replay mismatch".into()
        ))
    );
}
