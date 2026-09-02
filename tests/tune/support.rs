#![allow(dead_code)]

use sha2::{Digest, Sha256};

pub fn baseline_decision() -> Vec<u8> {
    outer_decision(&decision_payloads())
}

pub fn tuned_decision() -> Vec<u8> {
    outer_decision(&tuned_payloads())
}

fn tuned_payloads() -> Vec<Vec<u8>> {
    let mut payloads = decision_payloads();
    let pre_state = [0x82; 32];
    let post_state = [0xa4; 32];
    let (frontier, choice, plan_digest, frontier_digest) = tuned_frontier(pre_state, post_state);
    let candidates = tuned_candidates(plan_digest, &choice);
    let selection = tuned_selection(plan_digest);
    payloads[4] = frontier;
    payloads[5] = candidates.clone();
    payloads[6] = selection.clone();
    payloads[7] = tuned_replay(
        frontier_digest,
        pre_state,
        post_state,
        plan_digest,
        &candidates,
        &selection,
    );
    payloads
}

pub fn decision_payloads() -> Vec<Vec<u8>> {
    vec![
        identity_payload(),
        contract_payload(),
        workload_payload(),
        environment_payload(),
        frontier_payload(),
        candidates_payload(),
        selection_payload(),
        replay_payload(),
    ]
}

pub fn outer_decision(payloads: &[Vec<u8>]) -> Vec<u8> {
    assert_eq!(payloads.len(), 8);
    let mut bytes = b"CKTUNE01".to_vec();
    bytes.extend_from_slice(&1u32.to_be_bytes());
    for (index, payload) in payloads.iter().enumerate() {
        field(&mut bytes, u16::try_from(index + 1).expect("tag"), payload);
    }
    let digest = domain_hash(b"CK-TUNING-DECISION\0", &bytes);
    bytes.extend_from_slice(&digest);
    bytes
}

pub fn identity_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &text("0.14.0"));
    field(&mut payload, 2, &[0x02; 32]);
    field(&mut payload, 3, &text("rustc 1.90.0"));
    field(&mut payload, 4, &text("LLVM 22.1.8"));
    field(&mut payload, 5, &[0x05; 32]);
    for (tag, value) in (6u16..=15).zip([1u32, 1, 2, 3, 3, 3, 1, 5, 1, 1]) {
        field(&mut payload, tag, &value.to_be_bytes());
    }
    for tag in 16u16..=19 {
        field(&mut payload, tag, &[u8::try_from(tag).expect("byte"); 32]);
    }
    field(&mut payload, 20, &[1]);
    let mut target = Vec::new();
    field(&mut target, 1, &text("x86_64-unknown-linux-gnu"));
    field(&mut target, 2, &text("native"));
    field(&mut target, 3, &list(&[text("+sse2")]));
    field(&mut target, 4, &text("native-profile"));
    field(&mut payload, 21, &record(&target));
    field(&mut payload, 22, &[0]);
    payload
}

pub fn contract_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    for tag in 1u16..=5 {
        field(&mut payload, tag, &1u32.to_be_bytes());
    }
    field(&mut payload, 6, &[2]);
    for (tag, value) in (7u16..=11).zip([8u32, 4_096, 16, 8, 3]) {
        field(&mut payload, tag, &value.to_be_bytes());
    }
    field(&mut payload, 12, &1_800_000u64.to_be_bytes());
    for (tag, value) in (13u16..=14).zip([11u32, 10]) {
        field(&mut payload, tag, &value.to_be_bytes());
    }
    for (tag, value) in (15u16..=16).zip([50_000_000u64, 250_000_000]) {
        field(&mut payload, tag, &value.to_be_bytes());
    }
    for (tag, value) in (17u16..=31).zip([
        32u32, 3, 20, 3, 2_250, 4, 5, 6, 5, 16, 97, 100, 102, 100, 16,
    ]) {
        field(&mut payload, tag, &value.to_be_bytes());
    }
    let digest = domain_hash(b"CK-TUNE-POLICY\0", &record(&payload));
    field(&mut payload, 32, &digest);
    payload
}

fn workload_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &[0x31; 32]);
    field(&mut payload, 2, &[0x32; 32]);
    field(&mut payload, 3, &1_024u64.to_be_bytes());
    field(&mut payload, 4, &list(&[text("--protocol=1")]));
    field(&mut payload, 5, &list(&[]));
    field(&mut payload, 6, &1_000u32.to_be_bytes());
    field(&mut payload, 7, &list(&[]));
    field(
        &mut payload,
        8,
        &list(&[
            record(&case_identity("search", 1)),
            record(&case_identity("validation", 2)),
        ]),
    );
    payload
}

fn case_identity(id: &str, role: u8) -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &text(id));
    field(&mut payload, 2, &[role]);
    field(&mut payload, 3, &u64::from(role).to_be_bytes());
    field(&mut payload, 4, &1u32.to_be_bytes());
    field(&mut payload, 5, &[role; 32]);
    payload
}

fn environment_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    for (tag, value) in (1u16..=9).zip([
        "linux",
        "stable",
        "kernel",
        "x86_64",
        "vendor",
        "family",
        "model",
        "stepping",
        "microcode",
    ]) {
        field(&mut payload, tag, &text(value));
    }
    field(&mut payload, 10, &list(&[text("sse2")]));
    field(&mut payload, 11, &[1, 0, 0, 0, 4]);
    field(&mut payload, 12, &8u32.to_be_bytes());
    field(&mut payload, 13, &[0]);
    field(&mut payload, 14, &text("monotonic"));
    field(&mut payload, 15, &1u64.to_be_bytes());
    field(&mut payload, 16, &text("default"));
    field(
        &mut payload,
        17,
        &list(&[
            record(&calibration("search")),
            record(&calibration("validation")),
        ]),
    );
    field(&mut payload, 18, &[0x48; 32]);
    field(&mut payload, 19, &[0x49; 32]);
    payload
}

fn calibration(id: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &text(id));
    field(&mut payload, 2, &1u64.to_be_bytes());
    field(&mut payload, 3, &1u32.to_be_bytes());
    field(&mut payload, 4, &50_000_000u64.to_be_bytes());
    field(&mut payload, 5, &50_000_000u64.to_be_bytes());
    field(&mut payload, 6, &[0]);
    payload
}

fn frontier_payload() -> Vec<u8> {
    let sites = list(&[]);
    let units = list(&[]);
    let expansions = list(&[]);
    let candidate_space = candidate_space_digest(&sites, &units);
    let mut payload = Vec::new();
    field(&mut payload, 1, &candidate_space);
    field(&mut payload, 2, &sites);
    field(&mut payload, 3, &units);
    field(&mut payload, 4, &expansions);
    payload
}

fn tuned_frontier(
    pre_state: [u8; 32],
    post_state: [u8; 32],
) -> (Vec<u8>, Vec<u8>, [u8; 32], [u8; 32]) {
    let mut anchor = Vec::new();
    field(&mut anchor, 1, &text("kernel"));
    field(&mut anchor, 2, &[3]);
    field(&mut anchor, 3, &0u32.to_be_bytes());
    let mut root_material = Vec::new();
    field(&mut root_material, 1, &[18; 32]);
    field(&mut root_material, 2, &record(&anchor));
    let root_id = record_domain_hash(b"CK-TUNE-ROOT\0", &root_material);
    let mut site_material = Vec::new();
    field(&mut site_material, 1, &root_id);
    field(&mut site_material, 2, &[2]);
    field(&mut site_material, 3, &0u32.to_be_bytes());
    field(&mut site_material, 4, &pre_state);
    let site_id = record_domain_hash(b"CK-TUNE-SITE\0", &site_material);
    let mut site = Vec::new();
    field(&mut site, 1, &site_id);
    field(&mut site, 2, &[2]);
    field(&mut site, 3, &root_id);
    field(&mut site, 4, &pre_state);
    field(&mut site, 5, &0u32.to_be_bytes());
    field(&mut site, 6, &record(&anchor));
    let mut specialization = Vec::new();
    let mut binding = Vec::new();
    field(&mut binding, 1, &0u32.to_be_bytes());
    field(&mut binding, 2, &[1]);
    field(&mut binding, 3, &1u128.to_be_bytes());
    field(&mut specialization, 1, &list(&[record(&binding)]));
    field(&mut specialization, 2, &[0]);
    let mut alternative_payload = Vec::new();
    field(&mut alternative_payload, 1, &[2]);
    field(&mut alternative_payload, 2, &record(&specialization));
    let mut alternative = Vec::new();
    field(&mut alternative, 1, &site_id);
    let mut alternative_material = Vec::new();
    field(&mut alternative_material, 1, &site_id);
    field(&mut alternative_material, 2, &record(&alternative_payload));
    field(&mut alternative_material, 3, &post_state);
    let alternative_id = record_domain_hash(b"CK-TUNE-ALTERNATIVE\0", &alternative_material);
    field(&mut alternative, 2, &alternative_id);
    field(&mut alternative, 3, &pre_state);
    field(&mut alternative, 4, &post_state);
    field(&mut alternative, 5, &record(&alternative_payload));
    let site_alternatives = list(&[record(&alternative)]);
    let mut unit_material = Vec::new();
    let site_ids = list(&[site_id.to_vec()]);
    field(&mut unit_material, 1, &site_ids);
    field(&mut unit_material, 2, &pre_state);
    let unit_id = record_domain_hash(b"CK-TUNE-UNIT\0", &unit_material);
    let mut variant_material = Vec::new();
    field(&mut variant_material, 1, &unit_id);
    field(&mut variant_material, 2, &[2]);
    field(&mut variant_material, 3, &site_alternatives);
    field(&mut variant_material, 4, &10u64.to_be_bytes());
    field(&mut variant_material, 5, &20u64.to_be_bytes());
    field(&mut variant_material, 6, &30u64.to_be_bytes());
    field(&mut variant_material, 7, &post_state);
    let variant_id = record_domain_hash(b"CK-TUNE-UNIT-VARIANT\0", &variant_material);
    let mut variant = Vec::new();
    field(&mut variant, 1, &variant_id);
    field(&mut variant, 2, &[2]);
    field(&mut variant, 3, &site_alternatives);
    field(&mut variant, 4, &10u64.to_be_bytes());
    field(&mut variant, 5, &20u64.to_be_bytes());
    field(&mut variant, 6, &30u64.to_be_bytes());
    field(&mut variant, 7, &post_state);
    let mut unit = Vec::new();
    field(&mut unit, 1, &unit_id);
    field(&mut unit, 2, &site_ids);
    field(&mut unit, 3, &pre_state);
    field(&mut unit, 4, &list(&[record(&variant)]));
    let choice = plan_choice(unit_id, variant_id, pre_state, post_state);
    let plan_digest = domain_hash(b"CK-TUNE-PLAN\0", &list(&[record(&choice)]));
    let mut expansion = Vec::new();
    field(&mut expansion, 1, &0u32.to_be_bytes());
    field(&mut expansion, 2, &empty_plan_digest());
    field(&mut expansion, 3, &unit_id);
    field(&mut expansion, 4, &variant_id);
    field(&mut expansion, 5, &[1]);
    field(&mut expansion, 6, &optional(Some(&plan_digest)));
    field(&mut expansion, 7, &0u16.to_be_bytes());
    for tag in 8..=10 {
        field(&mut expansion, tag, &optional(Some(&10u64.to_be_bytes())));
    }
    let sites = list(&[record(&site)]);
    let units = list(&[record(&unit)]);
    let expansions = list(&[record(&expansion)]);
    let candidate_space = candidate_space_digest(&sites, &units);
    let frontier_digest = frontier_digest(candidate_space, &expansions);
    let mut payload = Vec::new();
    field(&mut payload, 1, &candidate_space);
    field(&mut payload, 2, &sites);
    field(&mut payload, 3, &units);
    field(&mut payload, 4, &expansions);
    (payload, choice, plan_digest, frontier_digest)
}

fn candidates_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &record(&baseline_candidate()));
    field(&mut payload, 2, &list(&[]));
    payload
}

fn tuned_candidates(plan_digest: [u8; 32], choice: &[u8]) -> Vec<u8> {
    let mut trial = Vec::new();
    field(&mut trial, 1, &plan_digest);
    field(&mut trial, 2, &list(&[record(choice)]));
    field(&mut trial, 3, &[0xb3; 32]);
    field(&mut trial, 4, &[0xb4; 32]);
    field(&mut trial, 5, &4_000u64.to_be_bytes());
    field(&mut trial, 6, &[8]);
    field(&mut trial, 7, &0u16.to_be_bytes());
    field(&mut trial, 8, &optional(Some(&[0x68; 32])));
    field(&mut trial, 9, &list(&[]));
    field(
        &mut trial,
        10,
        &record(&compile_cache_origin(
            plan_digest,
            [0xb3; 32],
            [0xb4; 32],
            4_000,
            [0xbc; 32],
        )),
    );
    field(&mut trial, 11, &optional(None));
    field(&mut trial, 12, &[0xbc; 32]);
    let mut payload = Vec::new();
    field(&mut payload, 1, &record(&baseline_candidate()));
    field(&mut payload, 2, &list(&[record(&trial)]));
    payload
}

fn plan_choice(
    unit_id: [u8; 32],
    variant_id: [u8; 32],
    pre_state: [u8; 32],
    post_state: [u8; 32],
) -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &unit_id);
    field(&mut payload, 2, &variant_id);
    field(&mut payload, 3, &[2]);
    field(&mut payload, 4, &pre_state);
    field(&mut payload, 5, &post_state);
    payload
}

fn baseline_candidate() -> Vec<u8> {
    let empty_plan = empty_plan_digest();
    let mut payload = Vec::new();
    field(&mut payload, 1, &empty_plan);
    field(&mut payload, 2, &list(&[]));
    field(&mut payload, 3, &[0x63; 32]);
    field(&mut payload, 4, &[0x64; 32]);
    field(&mut payload, 5, &4_096u64.to_be_bytes());
    field(&mut payload, 6, &[1]);
    field(&mut payload, 7, &0u16.to_be_bytes());
    field(&mut payload, 8, &optional(Some(&[0x68; 32])));
    field(&mut payload, 9, &list(&[]));
    field(
        &mut payload,
        10,
        &record(&compile_cache_origin(
            empty_plan, [0x63; 32], [0x64; 32], 4_096, [0x6c; 32],
        )),
    );
    field(&mut payload, 11, &optional(None));
    field(&mut payload, 12, &[0x6c; 32]);
    payload
}

fn cache_origin(key_digest: [u8; 32], entry_digest: [u8; 32]) -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &[1]);
    field(&mut payload, 2, &key_digest);
    field(&mut payload, 3, &entry_digest);
    payload
}

fn compile_cache_origin(
    plan_digest: [u8; 32],
    object_graph_digest: [u8; 32],
    link_recipe_digest: [u8; 32],
    primary_bytes: u64,
    primary_digest: [u8; 32],
) -> Vec<u8> {
    let mut key_material = Vec::new();
    field(&mut key_material, 1, &1u32.to_be_bytes());
    field(&mut key_material, 2, &record(&identity_payload()));
    field(&mut key_material, 3, &plan_digest);
    let key_digest = record_domain_hash(b"CK-TUNE-COMPILE-KEY\0", &key_material);
    let mut entry_material = Vec::new();
    field(&mut entry_material, 1, &key_digest);
    field(&mut entry_material, 2, &primary_digest);
    field(&mut entry_material, 3, &primary_bytes.to_be_bytes());
    field(&mut entry_material, 4, &object_graph_digest);
    field(&mut entry_material, 5, &link_recipe_digest);
    cache_origin(
        key_digest,
        record_domain_hash(b"CK-TUNE-COMPILE-ENTRY\0", &entry_material),
    )
}

fn measurement_cache_origin(candidates: &[u8], selection: &[u8]) -> Vec<u8> {
    let mut key_material = Vec::new();
    field(&mut key_material, 1, &1u32.to_be_bytes());
    field(&mut key_material, 2, &[0x48; 32]);
    field(&mut key_material, 3, &[0x49; 32]);
    let key_digest = record_domain_hash(b"CK-TUNE-MEASUREMENT-KEY\0", &key_material);
    let mut entry_material = Vec::new();
    field(&mut entry_material, 1, &key_digest);
    field(&mut entry_material, 2, &record(candidates));
    field(&mut entry_material, 3, &record(selection));
    cache_origin(
        key_digest,
        record_domain_hash(b"CK-TUNE-MEASUREMENT-ENTRY\0", &entry_material),
    )
}

fn selection_payload() -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &record(&round_summary(1)));
    field(&mut payload, 2, &record(&round_summary(2)));
    field(&mut payload, 3, &empty_plan_digest());
    field(&mut payload, 4, &[2]);
    field(&mut payload, 5, &optional(None));
    payload
}

fn tuned_selection(plan_digest: [u8; 32]) -> Vec<u8> {
    let mut certificate = Vec::new();
    for tag in 1..=8 {
        let value = if tag == 1 {
            plan_digest
        } else {
            [tag as u8; 32]
        };
        field(&mut certificate, tag, &value);
    }
    let mut payload = Vec::new();
    field(&mut payload, 1, &record(&round_summary(1)));
    field(&mut payload, 2, &record(&round_summary(2)));
    field(&mut payload, 3, &plan_digest);
    field(&mut payload, 4, &[1]);
    field(&mut payload, 5, &optional(Some(&record(&certificate))));
    payload
}

fn round_summary(round: u8) -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &[round]);
    field(&mut payload, 2, &list(&[]));
    field(&mut payload, 3, &list(&[]));
    payload
}

fn replay_payload() -> Vec<u8> {
    let sites = list(&[]);
    let units = list(&[]);
    let expansions = list(&[]);
    let digest = frontier_digest(candidate_space_digest(&sites, &units), &expansions);
    let mut payload = Vec::new();
    field(&mut payload, 1, &digest);
    field(&mut payload, 2, &[0x82; 32]);
    field(&mut payload, 3, &[0x82; 32]);
    field(&mut payload, 4, &[0x63; 32]);
    field(&mut payload, 5, &[0x64; 32]);
    field(
        &mut payload,
        6,
        &list(&[record(&output_identity("program"))]),
    );
    field(
        &mut payload,
        7,
        &record(&compile_cache_origin(
            empty_plan_digest(),
            [0x63; 32],
            [0x64; 32],
            4_096,
            [0x6c; 32],
        )),
    );
    field(
        &mut payload,
        8,
        &record(&measurement_cache_origin(
            &candidates_payload(),
            &selection_payload(),
        )),
    );
    field(&mut payload, 9, &[0x89; 32]);
    field(&mut payload, 10, &[0x8a; 32]);
    payload
}

fn tuned_replay(
    frontier_digest: [u8; 32],
    pre_state: [u8; 32],
    post_state: [u8; 32],
    plan_digest: [u8; 32],
    candidates: &[u8],
    selection: &[u8],
) -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &frontier_digest);
    field(&mut payload, 2, &pre_state);
    field(&mut payload, 3, &post_state);
    field(&mut payload, 4, &[0xb3; 32]);
    field(&mut payload, 5, &[0xb4; 32]);
    field(
        &mut payload,
        6,
        &list(&[record(&tuned_output_identity("program"))]),
    );
    field(
        &mut payload,
        7,
        &record(&compile_cache_origin(
            plan_digest,
            [0xb3; 32],
            [0xb4; 32],
            4_000,
            [0xbc; 32],
        )),
    );
    field(
        &mut payload,
        8,
        &record(&measurement_cache_origin(candidates, selection)),
    );
    field(&mut payload, 9, &[0x89; 32]);
    field(&mut payload, 10, &[0x8a; 32]);
    payload
}

fn output_identity(name: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &[1]);
    field(&mut payload, 2, &text(name));
    field(&mut payload, 3, &[0x6c; 32]);
    field(&mut payload, 4, &4_096u64.to_be_bytes());
    payload
}

fn tuned_output_identity(name: &str) -> Vec<u8> {
    let mut payload = Vec::new();
    field(&mut payload, 1, &[1]);
    field(&mut payload, 2, &text(name));
    field(&mut payload, 3, &[0xbc; 32]);
    field(&mut payload, 4, &4_000u64.to_be_bytes());
    payload
}

pub fn empty_plan_digest() -> [u8; 32] {
    domain_hash(b"CK-TUNE-PLAN\0", &0u32.to_be_bytes())
}

pub fn field(output: &mut Vec<u8>, tag: u16, payload: &[u8]) {
    output.extend_from_slice(&tag.to_be_bytes());
    output.extend_from_slice(
        &u32::try_from(payload.len())
            .expect("fixture field length")
            .to_be_bytes(),
    );
    output.extend_from_slice(payload);
}

pub fn text(value: &str) -> Vec<u8> {
    let mut bytes = u32::try_from(value.len())
        .expect("fixture text length")
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(value.as_bytes());
    bytes
}

pub fn record(fields: &[u8]) -> Vec<u8> {
    let mut bytes = u32::try_from(fields.len())
        .expect("fixture record length")
        .to_be_bytes()
        .to_vec();
    bytes.extend_from_slice(fields);
    bytes
}

pub fn list(items: &[Vec<u8>]) -> Vec<u8> {
    let mut bytes = u32::try_from(items.len())
        .expect("fixture list length")
        .to_be_bytes()
        .to_vec();
    for item in items {
        bytes.extend_from_slice(item);
    }
    bytes
}

pub fn optional(value: Option<&[u8]>) -> Vec<u8> {
    let mut bytes = vec![u8::from(value.is_some())];
    if let Some(value) = value {
        bytes.extend_from_slice(value);
    }
    bytes
}

pub fn resign(bytes: &mut [u8]) {
    let split = bytes.len() - 32;
    let (body, digest) = bytes.split_at_mut(split);
    digest.copy_from_slice(&domain_hash(b"CK-TUNING-DECISION\0", body));
}

fn domain_hash(domain: &[u8], bytes: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(bytes);
    hasher.finalize().into()
}

fn record_domain_hash(domain: &[u8], fields: &[u8]) -> [u8; 32] {
    domain_hash(domain, &record(fields))
}

fn candidate_space_digest(sites: &[u8], units: &[u8]) -> [u8; 32] {
    let mut material = Vec::new();
    field(&mut material, 1, sites);
    field(&mut material, 2, units);
    record_domain_hash(b"CK-TUNE-CANDIDATE-SPACE\0", &material)
}

fn frontier_digest(candidate_space: [u8; 32], expansions: &[u8]) -> [u8; 32] {
    let mut material = candidate_space.to_vec();
    material.extend_from_slice(expansions);
    domain_hash(b"CK-TUNE-FRONTIER\0", &material)
}
