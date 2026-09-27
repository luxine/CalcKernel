#[path = "../../benches/runtime_observation.rs"]
mod observer;

use std::{
    fs,
    sync::atomic::{AtomicUsize, Ordering},
};

fn output_path() -> std::path::PathBuf {
    static SERIAL: AtomicUsize = AtomicUsize::new(0);
    let dir = std::env::temp_dir().join(format!(
        "ck-runtime-observation-{}-{}",
        std::process::id(),
        SERIAL.fetch_add(1, Ordering::Relaxed)
    ));
    fs::create_dir(&dir).unwrap();
    dir.join("raw.jsonl")
}

#[test]
fn original_runtime_observation_should_preserve_every_result_and_bound_storage() {
    let path = output_path();
    let mut recorder = observer::Collector::new(&path, "{\"type\":\"identity\"}").unwrap();
    for i in 0..430 {
        let result = recorder.measure(i % 3, i < 9, || Ok::<u128, &str>(i as u128 + 1));
        assert_eq!(result, Ok(i as u128 + 1));
    }
    recorder.finish(true).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert_eq!(text.lines().count(), 431);
    assert!(text.contains("\"overflow\":true"));
    assert!(text.contains("\"gateNs\":429"));
    assert!(!text.contains("\"gateNs\":430"));
}

#[test]
fn original_runtime_observation_should_preserve_measurement_error() {
    let path = output_path();
    let mut recorder = observer::Collector::new(&path, "{\"type\":\"identity\"}").unwrap();
    let result = recorder.measure(1, false, || Err::<u128, _>("original kernel error"));
    assert_eq!(result, Err("original kernel error"));
    recorder.finish(false).unwrap();
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"gateNs\":null"));
    assert!(text.contains("\"samplingSucceeded\":false"));
}

#[test]
fn original_runtime_observation_should_never_overwrite_existing_evidence() {
    let path = output_path();
    fs::write(&path, "retain me").unwrap();
    assert!(observer::Collector::new(&path, "{}").is_err());
    assert_eq!(fs::read_to_string(path).unwrap(), "retain me");
}

#[test]
fn original_runtime_observation_should_encode_unavailable_values_and_json_strings() {
    let snapshot = observer::Snapshot::default().json();
    assert!(snapshot.contains("\"wallNs\":null"));
    assert!(snapshot.contains("\"systemCpuNs\":null"));
    assert_eq!(
        observer::json_string("a\n\"\\\t\u{1}"),
        "\"a\\n\\\"\\\\\\t\\u0001\""
    );
}

#[cfg(target_os = "linux")]
#[test]
fn original_runtime_observation_should_capture_linux_thread_resources() {
    let snapshot = observer::Snapshot::capture().json();
    for field in [
        "wallNs",
        "threadCpuNs",
        "cpu",
        "userCpuNs",
        "systemCpuNs",
        "minorFaults",
        "majorFaults",
        "voluntarySwitches",
        "involuntarySwitches",
    ] {
        assert!(
            !snapshot.contains(&format!("\"{field}\":null")),
            "{snapshot}"
        );
    }
}
