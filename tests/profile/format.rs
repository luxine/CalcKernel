use calckernel::{
    CkProfileCounter, CkProfileError, CkProfileSiteDescriptor, CkProfileSiteId, CkProfileSiteKind,
    parse_profile, parse_profile_shard, profile_site_table_digest, serialize_profile,
    serialize_profile_shard,
};
use sha2::{Digest, Sha256};

use super::{fixture_shard, fixture_shard_with_counter};

#[test]
fn profile_format_shard_should_round_trip_canonical_bytes() {
    let shard = fixture_shard(7, 19);
    let bytes = serialize_profile_shard(&shard).expect("serialize canonical shard");
    let parsed = parse_profile_shard(&bytes).expect("parse canonical shard");

    assert_eq!(parsed, shard);
}

#[test]
fn profile_format_final_should_round_trip_canonical_bytes() {
    let shard = fixture_shard(7, 19);
    let profile = calckernel::merge_profile_shards(&[shard]).expect("merge one shard");
    let bytes = serialize_profile(&profile).expect("serialize canonical profile");
    let parsed = parse_profile(&bytes).expect("parse canonical profile");

    assert_eq!(parsed, profile);
}

#[test]
fn profile_format_schema1_wire_digests_should_match_golden_vectors() {
    let shard = fixture_shard(7, 19);
    let shard_bytes = serialize_profile_shard(&shard).expect("serialize canonical shard");
    let profile = calckernel::merge_profile_shards(&[shard]).expect("merge canonical shard");
    let profile_bytes = serialize_profile(&profile).expect("serialize canonical profile");

    assert_eq!(
        sha256_hex(&shard_bytes),
        "10a583f4a75756d3461cb5ece71b2e6daac19ed4f3c6138280d4acafc1c9acff"
    );
    assert_eq!(
        sha256_hex(&profile_bytes),
        "c7b1ce6fe9c5c606124ece1349c4a44f165729d3ca133d64255163021ceb3626"
    );
}

#[test]
fn profile_format_parser_should_reject_bad_digest() {
    let mut bytes = serialize_profile_shard(&fixture_shard(7, 19)).expect("serialize shard");
    let last = bytes.last_mut().expect("digest byte");
    *last ^= 1;
    let error = parse_profile_shard(&bytes).expect_err("reject bad digest");

    assert_eq!(error, CkProfileError::DigestMismatch);
}

#[test]
fn profile_format_shard_parser_should_reject_terminal_profile() {
    let profile =
        calckernel::merge_profile_shards(&[fixture_shard(7, 19)]).expect("merge terminal profile");
    let bytes = serialize_profile(&profile).expect("serialize terminal profile");
    let error = parse_profile_shard(&bytes).expect_err("reject terminal merge input");

    assert_eq!(error, CkProfileError::UnexpectedMagic);
}

#[test]
fn profile_format_parser_should_reject_unknown_outer_tag() {
    let mut bytes = serialize_profile_shard(&fixture_shard(7, 19)).expect("serialize shard");
    bytes[12..14].copy_from_slice(&99u16.to_be_bytes());
    resign_shard(&mut bytes);
    let error = parse_profile_shard(&bytes).expect_err("reject unknown field");

    assert_eq!(
        error,
        CkProfileError::UnknownField {
            context: "outer",
            tag: 99,
        }
    );
}

#[test]
fn profile_format_parser_should_reject_noncanonical_outer_order() {
    let mut bytes = serialize_profile_shard(&fixture_shard(7, 19)).expect("serialize shard");
    bytes[12..14].copy_from_slice(&2u16.to_be_bytes());
    resign_shard(&mut bytes);
    let error = parse_profile_shard(&bytes).expect_err("reject noncanonical fields");

    assert_eq!(error, CkProfileError::NonCanonicalOrder("outer fields"));
}

#[test]
fn profile_format_parser_should_reject_declared_field_over_resource_limit() {
    let mut bytes = serialize_profile_shard(&fixture_shard(7, 19)).expect("serialize shard");
    bytes[14..18].copy_from_slice(&u32::MAX.to_be_bytes());
    resign_shard(&mut bytes);
    let error = parse_profile_shard(&bytes).expect_err("reject oversized declared field");

    assert_eq!(error, CkProfileError::ResourceLimit("outer field bytes"));
}

#[test]
fn profile_format_parser_should_reject_invalid_identity_utf8() {
    let mut bytes = serialize_profile_shard(&fixture_shard(7, 19)).expect("serialize shard");
    bytes[30] = 0xff;
    resign_shard(&mut bytes);
    let error = parse_profile_shard(&bytes).expect_err("reject invalid UTF-8");

    assert_eq!(error, CkProfileError::InvalidUtf8);
}

#[test]
fn profile_format_should_round_trip_histogram_payload() {
    let site = CkProfileSiteDescriptor {
        id: CkProfileSiteId([2; 16]),
        function_digest: [3; 32],
        location: 2,
        kind: CkProfileSiteKind::LoopTripHistogram { loop_identity: 9 },
    };
    let mut buckets = [0; 16];
    buckets[4] = 17;
    let shard = fixture_shard_with_counter(
        8,
        site,
        CkProfileCounter::Histogram {
            buckets,
            saturated: false,
        },
    );
    let bytes = serialize_profile_shard(&shard).expect("serialize histogram shard");

    assert_eq!(parse_profile_shard(&bytes), Ok(shard));
}

#[test]
fn profile_format_should_round_trip_candidate_constant_payload() {
    let site = CkProfileSiteDescriptor {
        id: CkProfileSiteId([3; 16]),
        function_digest: [4; 32],
        location: 3,
        kind: CkProfileSiteKind::CandidateConstant {
            decision_identity: 11,
            candidates: vec![-7, 0, 23],
        },
    };
    let shard = fixture_shard_with_counter(
        9,
        site,
        CkProfileCounter::CandidateConstant {
            candidates: vec![2, 5, 13],
            other: 17,
            saturated: false,
        },
    );
    let bytes = serialize_profile_shard(&shard).expect("serialize candidate shard");

    assert_eq!(parse_profile_shard(&bytes), Ok(shard));
}

#[test]
fn profile_format_parser_should_reject_trailing_bytes() {
    let mut bytes = serialize_profile_shard(&fixture_shard(7, 19)).expect("serialize shard");
    bytes.push(0);

    assert_eq!(
        parse_profile_shard(&bytes),
        Err(CkProfileError::DigestMismatch)
    );
}

#[test]
fn profile_format_should_reject_more_than_eight_candidate_constants() {
    let site = CkProfileSiteDescriptor {
        id: CkProfileSiteId([3; 16]),
        function_digest: [4; 32],
        location: 3,
        kind: CkProfileSiteKind::CandidateConstant {
            decision_identity: 1,
            candidates: (0..9).collect(),
        },
    };
    let error = profile_site_table_digest(&[site]).expect_err("reject candidate overflow");

    assert_eq!(error, CkProfileError::ResourceLimit("candidate constants"));
}

fn resign_shard(bytes: &mut [u8]) {
    let body_end = bytes.len() - 32;
    let mut hasher = Sha256::new();
    hasher.update(b"CK-PROFILE-SHARD\0");
    hasher.update(&bytes[..body_end]);
    bytes[body_end..].copy_from_slice(&hasher.finalize());
}

fn sha256_hex(bytes: &[u8]) -> String {
    let digest: [u8; 32] = Sha256::digest(bytes).into();
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}
