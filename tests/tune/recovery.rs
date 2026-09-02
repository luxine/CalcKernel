use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use calckernel::{
    PublicationFault, PublicationSet, TuneArtifactPaths, TuneOutputSet, TunePublishArtifacts,
    decode_tune_decision,
};

#[test]
fn recovery_preserves_impossible_digest_evidence_and_rejects_journal_free_backup() {
    let root = test_root("impossible");
    let paths = TuneArtifactPaths {
        primary: root.join("kernel.bin"),
        header: None,
        import_library: None,
    };
    let decision_path = root.join("kernel.cktune");
    fs::write(&paths.primary, b"old-primary").expect("old");
    fs::write(&decision_path, b"old-decision").expect("old decision");
    let output = TuneOutputSet::resolve(&paths, &decision_path, &[]).expect("output");
    let decision = decode_tune_decision(&super::support::baseline_decision()).expect("decision");
    let mut publication = PublicationSet::acquire_and_recover(output.clone()).expect("acquire");
    publication
        .publish_with_fault(
            &decision,
            TunePublishArtifacts {
                primary: b"new-primary".to_vec(),
                header: None,
                import_library: None,
            },
            PublicationFault::AfterPhase(calckernel::JournalPhase::Prepared),
        )
        .expect_err("crash");
    drop(publication);
    let stage = fs::read_dir(&root)
        .expect("scan")
        .filter_map(Result::ok)
        .find(|entry| {
            entry
                .file_name()
                .to_string_lossy()
                .ends_with("primary.stage")
        })
        .expect("stage")
        .path();
    fs::write(&stage, b"third-identity").expect("forge stage");
    assert!(PublicationSet::acquire_and_recover(output.clone()).is_err());
    assert!(stage.exists(), "evidence must be preserved");

    fs::remove_dir_all(&root).expect("reset root");
    fs::create_dir(&root).expect("recreate root");
    let fresh = TuneOutputSet::resolve(&paths, &decision_path, &[]).expect("fresh output");
    let set_hex = fresh
        .set_id()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let orphan = root.join(format!(
        ".ckc-tune-set-{set_hex}.11111111111111111111111111111111.primary.backup"
    ));
    fs::write(&orphan, b"orphan").expect("orphan backup");
    assert!(PublicationSet::acquire_and_recover(fresh).is_err());
    assert!(orphan.exists(), "orphan backup evidence must remain");
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn recovery_overlap_closure_recovers_an_intersecting_set_before_new_work() {
    let root = test_root("overlap");
    let shared = root.join("kernel.bin");
    fs::write(&shared, b"old").expect("old");
    let paths = TuneArtifactPaths {
        primary: shared.clone(),
        header: None,
        import_library: None,
    };
    let decision_a = root.join("a.cktune");
    let decision_b = root.join("b.cktune");
    fs::write(&decision_a, b"old-a").expect("old decision");
    let first = TuneOutputSet::resolve(&paths, &decision_a, &[]).expect("first");
    let decision = decode_tune_decision(&super::support::baseline_decision()).expect("decision");
    let mut publication = PublicationSet::acquire_and_recover(first).expect("acquire first");
    publication
        .publish_with_fault(
            &decision,
            TunePublishArtifacts {
                primary: b"new".to_vec(),
                header: None,
                import_library: None,
            },
            PublicationFault::AfterPhase(calckernel::JournalPhase::Prepared),
        )
        .expect_err("crash");
    drop(publication);

    let second = TuneOutputSet::resolve(&paths, &decision_b, &[]).expect("second");
    let recovered = PublicationSet::acquire_and_recover(second).expect("overlap recovery");
    drop(recovered);
    assert_eq!(fs::read(shared).expect("shared"), b"old");
    assert_eq!(fs::read(decision_a).expect("decision a"), b"old-a");
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn recovery_preserves_malformed_private_write_beside_a_valid_journal() {
    let root = test_root("malformed-write");
    let paths = TuneArtifactPaths {
        primary: root.join("kernel.bin"),
        header: None,
        import_library: None,
    };
    let decision_path = root.join("kernel.cktune");
    fs::write(&paths.primary, b"old").expect("old");
    fs::write(&decision_path, b"old-decision").expect("old decision");
    let output = TuneOutputSet::resolve(&paths, &decision_path, &[]).expect("output");
    let decision = decode_tune_decision(&super::support::baseline_decision()).expect("decision");
    let mut publication = PublicationSet::acquire_and_recover(output.clone()).expect("acquire");
    publication
        .publish_with_fault(
            &decision,
            TunePublishArtifacts {
                primary: b"new".to_vec(),
                header: None,
                import_library: None,
            },
            PublicationFault::AfterPhase(calckernel::JournalPhase::Prepared),
        )
        .expect_err("crash");
    drop(publication);

    let set_hex = output
        .set_id()
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect::<String>();
    let malformed = root.join(format!(".ckc-tune-set-{set_hex}.forged.write"));
    fs::write(&malformed, b"evidence").expect("malformed private write");
    assert!(PublicationSet::acquire_and_recover(output).is_err());
    assert!(malformed.exists(), "malformed evidence must not be deleted");
    fs::remove_dir_all(root).expect("cleanup");
}

fn test_root(label: &str) -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ckc-tune-recovery-{}-{label}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).expect("root");
    path
}
