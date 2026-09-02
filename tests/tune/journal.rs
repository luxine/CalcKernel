use std::{
    fs,
    sync::atomic::{AtomicU64, Ordering},
};

use calckernel::{
    JournalPhase, PublicationJournal, RecoveryDirection, TuneArtifactPaths, TuneOutputSet,
    TunePublishArtifacts, decode_publication_journal, encode_publication_journal,
};
use sha2::{Digest, Sha256};

#[test]
fn journal_schema_one_round_trips_exact_generation_layout_and_digest() {
    let root = test_root("roundtrip");
    let paths = TuneArtifactPaths {
        primary: root.join("kernel.bin"),
        header: Some(root.join("kernel.h")),
        import_library: Some(root.join("kernel.lib")),
    };
    fs::write(&paths.primary, b"old-primary").expect("old primary");
    let output = TuneOutputSet::resolve(&paths, &root.join("kernel.cktune"), &[]).expect("output");
    let artifacts = TunePublishArtifacts {
        primary: b"new-primary".to_vec(),
        header: Some(b"new-header".to_vec()),
        import_library: Some(b"new-import".to_vec()),
    };
    let journal = PublicationJournal::prepared(&output, [0x11; 16], b"decision", &artifacts)
        .expect("prepared journal");
    assert_eq!(
        (journal.phase(), journal.direction(), journal.generation()),
        (JournalPhase::Prepared, RecoveryDirection::Forward, 1)
    );
    let bytes = encode_publication_journal(&journal).expect("encode");
    assert!(bytes.len() <= 128 * 1024);
    assert_eq!(&bytes[..8], b"CKTJNL01");
    assert_eq!(&bytes[8..12], &1u32.to_be_bytes());
    assert_eq!(&bytes[12..20], &1u64.to_be_bytes());
    assert_eq!((bytes[20], bytes[21]), (1, 1));
    let mut digest = Sha256::new();
    digest.update(b"CK-TUNE-JOURNAL\0");
    digest.update(&bytes[..bytes.len() - 32]);
    assert_eq!(digest.finalize().as_slice(), &bytes[bytes.len() - 32..]);
    assert_eq!(decode_publication_journal(&bytes).expect("decode"), journal);

    let backed_up = journal.advance(JournalPhase::BackedUp).expect("successor");
    assert_eq!(backed_up.generation(), 2);
    let rollback = backed_up.begin_rollback().expect("rollback transition");
    assert_eq!(
        (rollback.direction(), rollback.generation()),
        (RecoveryDirection::Rollback, 3)
    );
    assert!(rollback.advance(JournalPhase::DecisionPublished).is_err());

    let mut corrupt = bytes.clone();
    corrupt[20] ^= 1;
    assert!(decode_publication_journal(&corrupt).is_err());
    let mut structurally_corrupt = bytes.clone();
    structurally_corrupt[20] = JournalPhase::Committed as u8;
    resign(&mut structurally_corrupt);
    assert!(decode_publication_journal(&structurally_corrupt).is_err());
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(decode_publication_journal(&trailing).is_err());
    fs::remove_dir_all(root).expect("cleanup");
}

fn resign(bytes: &mut [u8]) {
    let body_end = bytes.len() - 32;
    let mut digest = Sha256::new();
    digest.update(b"CK-TUNE-JOURNAL\0");
    digest.update(&bytes[..body_end]);
    bytes[body_end..].copy_from_slice(&digest.finalize());
}

fn test_root(label: &str) -> std::path::PathBuf {
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let path = std::env::temp_dir().join(format!(
        "ckc-tune-journal-{}-{label}-{}",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&path).expect("root");
    path
}
