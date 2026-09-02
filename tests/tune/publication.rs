use std::{
    fs,
    path::PathBuf,
    process::Command,
    sync::atomic::{AtomicU64, Ordering},
};

use calckernel::{
    PublicationFault, PublicationRole, PublicationSet, TuneArtifactPaths, TuneOutputSet,
    TunePublishArtifacts, decode_tune_decision,
};

#[test]
fn publication_destination_resolution_is_canonical_stable_and_alias_closed() {
    let root = test_root("destination");
    let paths = TuneArtifactPaths {
        primary: root.join("kernel.bin"),
        header: Some(root.join("kernel.h")),
        import_library: None,
    };
    let decision = root.join("kernel.cktune");
    let first = TuneOutputSet::resolve(&paths, &decision, &[]).expect("valid output set");
    let second = TuneOutputSet::resolve(&paths, &decision, &[]).expect("stable output set");
    assert_eq!(first.set_id(), second.set_id());
    assert_eq!(
        first
            .destinations()
            .iter()
            .map(|item| item.role)
            .collect::<Vec<_>>(),
        vec![
            PublicationRole::Decision,
            PublicationRole::Header,
            PublicationRole::Primary
        ]
    );
    assert!(
        first
            .destinations()
            .windows(2)
            .all(|items| items[0].destination_id != items[1].destination_id)
    );

    let duplicate = TuneArtifactPaths {
        primary: decision.clone(),
        header: None,
        import_library: None,
    };
    assert!(TuneOutputSet::resolve(&duplicate, &decision, &[]).is_err());
    assert!(TuneOutputSet::resolve(&paths, &root.join(".ckc-tune-forged"), &[]).is_err());
    assert!(TuneOutputSet::resolve(&paths, &root.join("CON"), &[]).is_err());
    assert!(TuneOutputSet::resolve(&paths, &root.join("nested/decision.cktune"), &[]).is_err());
    assert!(
        TuneOutputSet::resolve(&paths, &decision, std::slice::from_ref(&paths.primary)).is_err()
    );

    fs::write(&paths.primary, b"old").expect("seed primary");
    let alias = root.join("alias.bin");
    fs::hard_link(&paths.primary, &alias).expect("hardlink fixture");
    let aliased_paths = TuneArtifactPaths {
        primary: paths.primary.clone(),
        header: Some(alias),
        import_library: None,
    };
    assert!(TuneOutputSet::resolve(&aliased_paths, &decision, &[]).is_err());

    #[cfg(unix)]
    {
        std::os::unix::fs::symlink(&root, root.join("linked-parent")).expect("parent symlink");
        let linked_paths = TuneArtifactPaths {
            primary: root.join("linked-parent/linked.bin"),
            header: None,
            import_library: None,
        };
        assert!(
            TuneOutputSet::resolve(
                &linked_paths,
                &root.join("linked-parent/linked.cktune"),
                &[]
            )
            .is_err(),
            "every parent component must be resolved no-follow"
        );
    }
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn publication_locks_are_full_identity_owner_only_and_persistent() {
    let root = test_root("locks");
    let paths = TuneArtifactPaths {
        primary: root.join("kernel.bin"),
        header: None,
        import_library: None,
    };
    let output =
        TuneOutputSet::resolve(&paths, &root.join("kernel.cktune"), &[]).expect("output set");
    {
        let _publication =
            PublicationSet::acquire_and_recover(output.clone()).expect("acquire locks");
        let lock_files = fs::read_dir(&root)
            .expect("scan locks")
            .filter_map(|entry| {
                let entry = entry.ok()?;
                let name = entry.file_name().into_string().ok()?;
                name.ends_with(".lock").then_some((name, entry.path()))
            })
            .collect::<Vec<_>>();
        assert_eq!(lock_files.len(), 2);
        for (name, path) in lock_files {
            assert!(name.starts_with(".ckc-tune-dest-") && name.len() == 15 + 64 + 5);
            let bytes = fs::read(path).expect("lock bytes");
            assert_eq!(&bytes[..8], b"CKTLCK01");
            assert_eq!(bytes.len(), 40);
        }
    }
    assert_eq!(
        fs::read_dir(&root)
            .expect("persistent scan")
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".lock"))
            .count(),
        2
    );
    let _again = PublicationSet::acquire_and_recover(output).expect("persistent locks reopen");
    drop(_again);
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn publication_crash_matrix_recovers_only_complete_old_or_new_sets() {
    let faults = [
        PublicationFault::AfterStages,
        PublicationFault::AfterJournalPrivate(calckernel::JournalPhase::Prepared),
        PublicationFault::AfterJournalUpdate(calckernel::JournalPhase::Prepared),
        PublicationFault::AfterPhase(calckernel::JournalPhase::Prepared),
        PublicationFault::AfterBackups,
        PublicationFault::AfterJournalPrivate(calckernel::JournalPhase::BackedUp),
        PublicationFault::AfterJournalUpdate(calckernel::JournalPhase::BackedUp),
        PublicationFault::AfterPhase(calckernel::JournalPhase::BackedUp),
        PublicationFault::AfterDecision,
        PublicationFault::AfterJournalPrivate(calckernel::JournalPhase::DecisionPublished),
        PublicationFault::AfterJournalUpdate(calckernel::JournalPhase::DecisionPublished),
        PublicationFault::AfterPhase(calckernel::JournalPhase::DecisionPublished),
        PublicationFault::AfterSidecars,
        PublicationFault::AfterJournalPrivate(calckernel::JournalPhase::SidecarsPublished),
        PublicationFault::AfterJournalUpdate(calckernel::JournalPhase::SidecarsPublished),
        PublicationFault::AfterPhase(calckernel::JournalPhase::SidecarsPublished),
        PublicationFault::AfterPrimary,
        PublicationFault::AfterJournalPrivate(calckernel::JournalPhase::PrimaryPublished),
        PublicationFault::AfterJournalUpdate(calckernel::JournalPhase::PrimaryPublished),
        PublicationFault::AfterPhase(calckernel::JournalPhase::PrimaryPublished),
        PublicationFault::AfterJournalPrivate(calckernel::JournalPhase::Committed),
        PublicationFault::AfterJournalUpdate(calckernel::JournalPhase::Committed),
        PublicationFault::AfterPhase(calckernel::JournalPhase::Committed),
    ];
    for fault in faults {
        let root = test_root("crash");
        let paths = TuneArtifactPaths {
            primary: root.join("kernel.bin"),
            header: Some(root.join("kernel.h")),
            import_library: None,
        };
        let decision_path = root.join("kernel.cktune");
        fs::write(&paths.primary, b"old-primary").expect("old primary");
        fs::write(paths.header.as_ref().expect("header"), b"old-header").expect("old header");
        fs::write(&decision_path, b"old-decision").expect("old decision");
        let output = TuneOutputSet::resolve(&paths, &decision_path, &[]).expect("output");
        let decision =
            decode_tune_decision(&super::support::baseline_decision()).expect("decision");
        let artifacts = TunePublishArtifacts {
            primary: b"new-primary".to_vec(),
            header: Some(b"new-header".to_vec()),
            import_library: None,
        };
        let mut publication = PublicationSet::acquire_and_recover(output.clone()).expect("acquire");
        assert!(
            publication
                .publish_with_fault(&decision, artifacts.clone(), fault)
                .is_err(),
            "fault {fault:?}"
        );
        drop(publication);

        let recovered = PublicationSet::acquire_and_recover(output).expect("recover");
        drop(recovered);
        let expect_new = matches!(
            fault,
            PublicationFault::AfterPrimary
                | PublicationFault::AfterJournalPrivate(
                    calckernel::JournalPhase::PrimaryPublished
                        | calckernel::JournalPhase::Committed
                )
                | PublicationFault::AfterJournalUpdate(
                    calckernel::JournalPhase::PrimaryPublished
                        | calckernel::JournalPhase::Committed
                )
                | PublicationFault::AfterPhase(
                    calckernel::JournalPhase::PrimaryPublished
                        | calckernel::JournalPhase::Committed
                )
        );
        assert_eq!(
            fs::read(&paths.primary).expect("primary"),
            if expect_new {
                b"new-primary".as_slice()
            } else {
                b"old-primary".as_slice()
            },
            "fault {fault:?}"
        );
        assert_eq!(
            fs::read(paths.header.as_ref().expect("header")).expect("header"),
            if expect_new {
                b"new-header".as_slice()
            } else {
                b"old-header".as_slice()
            },
            "fault {fault:?}"
        );
        assert_eq!(
            fs::read(&decision_path).expect("decision"),
            if expect_new {
                super::support::baseline_decision()
            } else {
                b"old-decision".to_vec()
            },
            "fault {fault:?}"
        );
        assert!(
            fs::read_dir(&root)
                .expect("scan")
                .filter_map(Result::ok)
                .map(|entry| entry.file_name().to_string_lossy().into_owned())
                .all(|name| name.ends_with(".lock") || !name.starts_with(".ckc-tune-set-")),
            "reserved transaction debris after {fault:?}"
        );
        fs::remove_dir_all(root).expect("cleanup");
    }
}

#[test]
fn publication_success_is_primary_last_and_leaves_only_persistent_locks() {
    let root = test_root("success");
    let paths = TuneArtifactPaths {
        primary: root.join("kernel.bin"),
        header: None,
        import_library: None,
    };
    let decision_path = root.join("kernel.cktune");
    let output = TuneOutputSet::resolve(&paths, &decision_path, &[]).expect("output");
    let decision = decode_tune_decision(&super::support::baseline_decision()).expect("decision");
    let artifacts = TunePublishArtifacts {
        primary: b"new-primary".to_vec(),
        header: None,
        import_library: None,
    };
    let mut publication = PublicationSet::acquire_and_recover(output).expect("acquire");
    publication
        .publish_verified(&decision, artifacts)
        .expect("publish");
    drop(publication);
    assert_eq!(fs::read(&paths.primary).expect("primary"), b"new-primary");
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_ne!(
            fs::metadata(&paths.primary)
                .expect("primary metadata")
                .permissions()
                .mode()
                & 0o100,
            0,
            "an executable primary must remain directly runnable"
        );
    }
    assert_eq!(
        fs::read(decision_path).expect("decision"),
        super::support::baseline_decision()
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn publication_namespace_revalidation_ignores_expected_recovery_identity_changes() {
    let root = test_root("revalidated");
    let paths = TuneArtifactPaths {
        primary: root.join("kernel.bin"),
        header: None,
        import_library: None,
    };
    let decision_path = root.join("kernel.cktune");
    fs::write(&paths.primary, b"old-primary").expect("old primary");
    fs::write(&decision_path, b"old-decision").expect("old decision");
    let original = TuneOutputSet::resolve(&paths, &decision_path, &[]).expect("original");
    let decision = decode_tune_decision(&super::support::baseline_decision()).expect("decision");
    let artifacts = TunePublishArtifacts {
        primary: b"new-primary".to_vec(),
        header: None,
        import_library: None,
    };
    let mut first = PublicationSet::acquire_and_recover(original).expect("first acquire");
    first
        .publish_with_fault(
            &decision,
            artifacts.clone(),
            PublicationFault::AfterPhase(calckernel::JournalPhase::BackedUp),
        )
        .expect_err("simulated crash");
    drop(first);

    let absent_snapshot =
        TuneOutputSet::resolve(&paths, &decision_path, &[]).expect("absent snapshot");
    let mut recovered =
        PublicationSet::acquire_and_recover(absent_snapshot).expect("recover while acquiring");
    recovered
        .publish_verified(&decision, artifacts)
        .expect("recovery identity changes are not namespace changes");
    drop(recovered);
    assert_eq!(fs::read(paths.primary).expect("primary"), b"new-primary");
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
fn publication_real_process_death_recovers_a_complete_set() {
    let root = test_root("killed-session");
    let status = Command::new(std::env::current_exe().expect("test executable"))
        .args([
            "--exact",
            "publication::publication_killed_session_child",
            "--ignored",
        ])
        .env("CK_TUNE_KILL_ROOT", &root)
        .status()
        .expect("spawn crash child");
    assert!(!status.success(), "child must terminate abnormally");

    let paths = TuneArtifactPaths {
        primary: root.join("kernel.bin"),
        header: None,
        import_library: None,
    };
    let decision_path = root.join("kernel.cktune");
    let output = TuneOutputSet::resolve(&paths, &decision_path, &[]).expect("output");
    drop(PublicationSet::acquire_and_recover(output).expect("recover killed session"));
    assert_eq!(fs::read(paths.primary).expect("primary"), b"new-primary");
    assert_eq!(
        fs::read(decision_path).expect("decision"),
        super::support::baseline_decision()
    );
    fs::remove_dir_all(root).expect("cleanup");
}

#[test]
#[ignore = "private helper launched by publication_real_process_death_recovers_a_complete_set"]
fn publication_killed_session_child() {
    let root = PathBuf::from(std::env::var_os("CK_TUNE_KILL_ROOT").expect("crash root"));
    let paths = TuneArtifactPaths {
        primary: root.join("kernel.bin"),
        header: None,
        import_library: None,
    };
    let decision_path = root.join("kernel.cktune");
    fs::write(&paths.primary, b"old-primary").expect("old primary");
    fs::write(&decision_path, b"old-decision").expect("old decision");
    let output = TuneOutputSet::resolve(&paths, &decision_path, &[]).expect("output");
    let decision = decode_tune_decision(&super::support::baseline_decision()).expect("decision");
    let mut publication = PublicationSet::acquire_and_recover(output).expect("acquire");
    publication
        .publish_with_fault(
            &decision,
            TunePublishArtifacts {
                primary: b"new-primary".to_vec(),
                header: None,
                import_library: None,
            },
            PublicationFault::AfterPrimary,
        )
        .expect_err("stop after primary rename");
    std::process::abort();
}

fn test_root(label: &str) -> PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ckc-tune-pub-{}-{label}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).expect("create root");
    path
}
