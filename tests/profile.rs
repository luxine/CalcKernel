#[path = "profile/format.rs"]
mod format;
#[path = "profile/generation.rs"]
mod generation;
#[path = "profile/inspection.rs"]
mod inspection;
#[path = "profile/merge.rs"]
mod merge;

use std::sync::atomic::{AtomicU64, Ordering};

use calckernel::{
    CkCompilerProfileIdentity, CkModuleProfileIdentity, CkProfileContract, CkProfileCounter,
    CkProfileCounterRecord, CkProfileCpuPolicy, CkProfileEndianness, CkProfileIdentity,
    CkProfileModes, CkProfileObjectFormat, CkProfileOptimizationFamily, CkProfileSchemaIdentity,
    CkProfileShard, CkProfileSiteDescriptor, CkProfileSiteId, CkProfileSiteKind,
    CkProfileTargetIdentity, CkProfileTopology,
};

static NEXT_TEST_ROOT: AtomicU64 = AtomicU64::new(0);

fn fixture_identity(site_table_digest: [u8; 32]) -> CkProfileIdentity {
    CkProfileIdentity {
        compiler: CkCompilerProfileIdentity {
            package_version: "0.13.0-test".to_string(),
            source_identity: [0x11; 32],
            profile_runtime_identity: [0x12; 32],
        },
        module: CkModuleProfileIdentity {
            semantic_graph_digest: [0x21; 32],
            pre_profile_kir_digest: [0x22; 32],
            site_table_digest,
        },
        schemas: CkProfileSchemaIdentity {
            language: 1,
            native_abi: 1,
            runtime_abi: 2,
            kir: 3,
            proof: 3,
            cost_model: 3,
            target_profile: 1,
            llvm_bridge: 4,
            cache: 4,
        },
        target: CkProfileTargetIdentity {
            triple: "x86_64-unknown-linux-gnu".to_string(),
            pointer_width: 64,
            endianness: CkProfileEndianness::Little,
            object_format: CkProfileObjectFormat::Elf,
            os_abi: "linux-gnu".to_string(),
            target_set_digest: [0x31; 32],
        },
        modes: CkProfileModes {
            overflow_checked: true,
            bounds_checked: true,
            strict_float: true,
            sanitizer: false,
            topology: CkProfileTopology::NativeLibrary,
            optimization_family: CkProfileOptimizationFamily::O3,
            cpu_policy: CkProfileCpuPolicy::Baseline,
        },
        contract: CkProfileContract::schema1(),
    }
}

fn fixture_site(byte: u8) -> CkProfileSiteDescriptor {
    CkProfileSiteDescriptor {
        id: CkProfileSiteId([byte; 16]),
        function_digest: [byte.wrapping_add(1); 32],
        location: u32::from(byte),
        kind: CkProfileSiteKind::FunctionEntry,
    }
}

fn fixture_shard(run: u8, count: u64) -> CkProfileShard {
    let site = fixture_site(1);
    fixture_shard_with_counter(run, site, CkProfileCounter::Scalar(count))
}

fn fixture_shard_with_counter(
    run: u8,
    site: CkProfileSiteDescriptor,
    counter: CkProfileCounter,
) -> CkProfileShard {
    let site_table_digest =
        calckernel::profile_site_table_digest(std::slice::from_ref(&site)).expect("site digest");
    CkProfileShard {
        identity: fixture_identity(site_table_digest),
        sites: vec![site.clone()],
        counters: vec![CkProfileCounterRecord {
            site_id: site.id,
            counter,
        }],
        run_id: [run; 16],
        overflowed: false,
        incomplete_observations: false,
    }
}

fn test_root(label: &str) -> std::path::PathBuf {
    let serial = NEXT_TEST_ROOT.fetch_add(1, Ordering::Relaxed);
    std::env::current_dir()
        .expect("current test directory")
        .join("target")
        .join("profile-tests")
        .join(format!("{label}-{}-{serial}", std::process::id()))
}
