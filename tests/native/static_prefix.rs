use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Output},
};

use sha2::{Digest, Sha256};

use super::support::{
    oracle::{clang_oracle_22, repo_root},
    temp::temp_dir,
};

struct CoffFixture {
    root: PathBuf,
    clang: PathBuf,
    prefix: PathBuf,
}

impl CoffFixture {
    fn new() -> Self {
        let root = temp_dir("ckc-real-coff-crt");
        fs::create_dir_all(&root).unwrap();
        Self {
            root,
            clang: clang_oracle_22().expect("CRT regression requires the pinned Clang oracle"),
            prefix: std::env::var_os("CKC_LLVM_PREFIX")
                .expect("pinned LLVM prefix")
                .into(),
        }
    }

    fn tool(&self, name: &str) -> PathBuf {
        self.prefix
            .join("bin")
            .join(format!("{name}{}", std::env::consts::EXE_SUFFIX))
    }

    fn object(&self, target: &str, label: &str, flag: &str, mismatch: Option<&str>) -> PathBuf {
        let source = self.root.join(format!("{label}.cpp"));
        let object = source.with_extension("obj");
        let directive = mismatch.map_or_else(String::new, |value| {
            format!("#pragma detect_mismatch(\"RuntimeLibrary\", \"{value}\")\n")
        });
        fs::write(
            &source,
            format!("{directive}int fixture() {{ return 7; }}\n"),
        )
        .unwrap();
        success(
            Command::new(&self.clang)
                .arg("--driver-mode=cl")
                .arg(format!("--target={target}"))
                .args(["/nologo", "/c", flag])
                .arg(format!("/Fo{}", object.display()))
                .arg(source)
                .output()
                .expect("emit real COFF object"),
        );
        object
    }

    fn archive(&self, label: &str, objects: &[PathBuf]) -> PathBuf {
        let archive = self.root.join(format!("{label}.lib"));
        success(
            Command::new(self.tool("llvm-ar"))
                .arg("rcs")
                .arg(&archive)
                .args(objects)
                .output()
                .expect("archive real COFF members"),
        );
        archive
    }

    fn check(&self, archive: &Path, readobj: &Path) -> Output {
        Command::new("pwsh")
            .args(["-NoLogo", "-NoProfile", "-Command", "$ErrorActionPreference = 'Stop'; . $env:CKC_TEST_CRT_SCRIPT; Assert-MsvcStaticArchives -ReadObj $env:CKC_TEST_READOBJ -Archives @($env:CKC_TEST_ARCHIVE)"])
            .env("CKC_TEST_CRT_SCRIPT", repo_root().join("scripts/validate-msvc-crt.ps1"))
            .env("CKC_TEST_READOBJ", readobj)
            .env("CKC_TEST_ARCHIVE", archive)
            .output().expect("validate actual COFF directives")
    }
}

impl Drop for CoffFixture {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.root);
    }
}

fn success(output: Output) {
    assert!(
        output.status.success(),
        "stdout={}\nstderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn rejected(output: Output, reason: &str) {
    assert!(!output.status.success(), "accepted invalid CRT input");
    assert!(
        String::from_utf8_lossy(&output.stderr).contains(reason),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

fn coff_target() -> &'static str {
    match std::env::consts::ARCH {
        "x86_64" => "x86_64-pc-windows-msvc",
        "aarch64" => "aarch64-pc-windows-msvc",
        arch => panic!("unsupported release host architecture {arch}"),
    }
}

#[test]
fn windows_static_crt_should_check_real_coff_directives_for_the_host_architecture() {
    let fixture = CoffFixture::new();
    let readobj = fixture.tool("llvm-readobj");
    let arch = std::env::consts::ARCH;
    let target = coff_target();
    {
        let mt = fixture.object(
            target,
            &format!("{arch}-mt"),
            "/MT",
            Some("MT_StaticRelease"),
        );
        let good = fixture.archive(&format!("{arch}-static"), std::slice::from_ref(&mt));
        success(fixture.check(&good, &readobj));
        // Without CRT headers, Clang still emits genuine DEFAULTLIB directives.
        let plain = fixture.object(target, &format!("{arch}-plain"), "/MT", None);
        success(fixture.check(
            &fixture.archive(&format!("{arch}-plain"), &[plain]),
            &readobj,
        ));
        for (label, flag, mismatch) in [
            ("dynamic", "/MD", Some("MD_DynamicRelease")),
            ("defaultlib-only", "/MD", None),
            ("mismatch-only", "/MT", Some("MD_DynamicRelease")),
            ("static-debug", "/MTd", Some("MTd_StaticDebug")),
            ("dynamic-debug", "/MDd", None),
        ] {
            let label = format!("{arch}-{label}");
            let object = fixture.object(target, &label, flag, mismatch);
            let bad = fixture.archive(&label, std::slice::from_ref(&object));
            rejected(fixture.check(&bad, &readobj), "non-release-static CRT");
            let mixed = fixture.archive(&format!("{label}-mixed"), &[mt.clone(), object]);
            rejected(fixture.check(&mixed, &readobj), "non-release-static CRT");
        }
    }
}

#[test]
fn windows_static_crt_should_reject_missing_broken_and_empty_inputs() {
    let fixture = CoffFixture::new();
    let readobj = fixture.tool("llvm-readobj");
    let empty = fixture.archive("empty", &[]);
    rejected(
        fixture.check(&empty, &readobj),
        "no release-static CRT evidence",
    );
    rejected(
        fixture.check(&empty, &fixture.root.join("missing-readobj")),
        "missing llvm-readobj",
    );
    let corrupt = fixture.root.join("corrupt.lib");
    fs::write(&corrupt, b"not an archive").unwrap();
    rejected(fixture.check(&corrupt, &readobj), "llvm-readobj failed");
    rejected(
        fixture.check(&fixture.root.join("missing.lib"), &readobj),
        "missing static archive",
    );
}

#[test]
fn windows_static_prefix_should_reject_dynamic_crt_despite_a_static_manifest() {
    let fixture = CoffFixture::new();
    let root = fixture.root.join("prefix");
    for directory in ["bin", "lib", "share/ckc/runtime"] {
        fs::create_dir_all(root.join(directory)).unwrap();
    }
    for tool in ["llvm-config", "llvm-readobj"] {
        fs::copy(fixture.tool(tool), root.join(format!("bin/{tool}.exe"))).unwrap();
    }
    let target = coff_target();
    let mt = fixture.object(target, "static", "/MT", Some("MT_StaticRelease"));
    let archive = fixture.archive("static", &[mt]);
    let libraries = [
        "lldCOFF",
        "lldCommon",
        "LLVMDTLTO",
        "LLVMLibDriver",
        "LLVMWindowsManifest",
    ];
    for library in libraries {
        fs::copy(&archive, root.join(format!("lib/{library}.lib"))).unwrap();
    }
    // Runtime bytes are hash fixtures; CRT assertions use only real COFF archives above.
    let objects = [
        "runtime.obj",
        "format_int.obj",
        "format_float.obj",
        "ryu.obj",
        "platform.obj",
    ];
    for object in objects.into_iter().chain(["kernel32.lib"]) {
        fs::write(
            root.join("share/ckc/runtime").join(object),
            b"runtime hash fixture",
        )
        .unwrap();
    }
    let hash = format!("{:x}", Sha256::digest(b"runtime hash fixture"));
    let jit_support = if target == "x86_64-pc-windows-msvc" {
        fs::write(
            root.join("share/ckc/runtime/jit_image_base.obj"),
            b"runtime hash fixture",
        )
        .unwrap();
        format!(
            "runtime_jit_support = \"jit_image_base.obj\"\nruntime_jit_support_sha256 = \"{hash}\"\n"
        )
    } else {
        String::new()
    };
    let names = objects.map(|name| format!("\"{name}\"")).join(", ");
    let hashes = vec![format!("\"{hash}\""); 5].join(", ");
    let libraries = libraries.map(|name| format!("\"{name}\"")).join(", ");
    let manifest = format!(
        "schema = 1\nversion = \"22.1.8\"\ntarget = \"{target}\"\nprofile = \"release\"\nsource_sha256 = \"922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888\"\nstatic_only = true\nmsvc_runtime_library = \"MultiThreaded\"\nstatic_libraries = [{libraries}]\nruntime_objects = [{names}]\nruntime_sha256 = [{hashes}]\nruntime_platform_import = \"kernel32.lib\"\nruntime_platform_import_sha256 = \"{hash}\"\n{jit_support}"
    );
    let path = root.join("share/ckc/llvm-build.toml");
    fs::write(&path, &manifest).unwrap();
    let run = || {
        Command::new("pwsh")
            .args(["-NoLogo", "-NoProfile", "-File"])
            .arg(repo_root().join("scripts/validate-llvm-prefix.ps1"))
            .arg("-Prefix")
            .arg(&root)
            .args(["-Target", target, "-Profile", "release"])
            .output()
            .expect("run production prefix verifier")
    };
    success(run());
    let md = fixture.object(target, "dynamic", "/MD", Some("MD_DynamicRelease"));
    fs::copy(
        fixture.archive("dynamic", &[md]),
        root.join("lib/LLVMDTLTO.lib"),
    )
    .unwrap();
    rejected(run(), "non-release-static CRT");
    fs::copy(&archive, root.join("lib/LLVMDTLTO.lib")).unwrap();
    success(run());
    for missing in ["LLVMLibDriver", "LLVMWindowsManifest"] {
        fs::write(
            &path,
            manifest
                .replace(&format!("\"{missing}\", "), "")
                .replace(&format!(", \"{missing}\""), ""),
        )
        .unwrap();
        rejected(run(), "missing static COFF component");
    }
    fs::write(&path, manifest.replace("MultiThreaded", "MultiThreadedDLL")).unwrap();
    rejected(run(), "MSVC runtime library mismatch");
}
