//! Retain original failure bytes without running, repairing or accepting them.

use std::{
    ffi::{OsStr, OsString},
    fs::File,
    io::{self, Write},
    os::unix::{ffi::OsStrExt, process::ExitStatusExt},
    path::Path,
    process::Output,
};

use sha2::{Digest, Sha256};

#[path = "tune_capture/files.rs"]
mod files;

use files::{Copied, Directory, Source, hex};

pub(super) struct Request<'a> {
    pub(super) root: &'a Path,
    pub(super) compiler: &'a Path,
    pub(super) source: &'a Path,
    pub(super) manifest: &'a Path,
    pub(super) runner: &'a Path,
    pub(super) compile_cache: &'a Path,
    pub(super) arguments: &'a [OsString],
    pub(super) original: &'a Output,
    pub(super) identity_context: Result<&'a [u8], &'a str>,
}

#[derive(Clone, Copy)]
pub(super) struct Limits {
    pub(super) compiler_bytes: u64,
    pub(super) file_bytes: u64,
    pub(super) total_bytes: u64,
    pub(super) cache_files: usize,
}

impl Limits {
    pub(super) const DEFAULT: Self = Self {
        compiler_bytes: 256 * 1024 * 1024,
        file_bytes: 16 * 1024 * 1024,
        total_bytes: 384 * 1024 * 1024,
        cache_files: 64,
    };
}

pub(super) fn capture(request: Request<'_>, limits: Limits) -> io::Result<()> {
    if request.original.status.success() {
        return Ok(());
    }
    let root = Directory::open(request.root)?;
    let observations = root.create_directory("post-failure-startup", true)?;
    let destination = observations.create_directory("failed-original", false)?;
    let mut report = Report {
        file: destination.create_file(OsStr::new("manifest.txt"))?,
        complete: true,
        remaining: limits.total_bytes,
    };
    writeln!(
        report.file,
        "CKTUNE-FAILURE-CAPTURE/1\ndiagnosticOnly=true\nfailedPlanIdentityVerified=false\ncaptureState=started\nrootHex={}\noriginalRawWaitStatus={}\ncompilerByteLimit={}\nfileByteLimit={}\ntotalByteLimit={}\ncacheEntryLimit={}",
        hex(request.root.as_os_str().as_bytes()),
        request.original.status.into_raw(),
        limits.compiler_bytes,
        limits.file_bytes,
        limits.total_bytes,
        limits.cache_files,
    )?;
    report.file.sync_all()?;

    // Original streams precede optional large files. A failed compiler copy
    // must not discard the status or the failure which prompted this capture.
    for (label, bytes) in [
        ("original.stdout", request.original.stdout.as_slice()),
        ("original.stderr", request.original.stderr.as_slice()),
    ] {
        report.memory(&destination, label, bytes, limits.file_bytes)?;
    }
    match request.identity_context {
        Ok(bytes) => report.memory(
            &destination,
            "identity-context.txt",
            bytes,
            limits.file_bytes,
        )?,
        Err(error) => report.error("identity-context.txt", &io::Error::other(error))?,
    }
    let arguments = arguments_hex(request.arguments, limits.file_bytes);
    match arguments {
        Ok(bytes) => report.memory(&destination, "arguments.hex", &bytes, limits.file_bytes)?,
        Err(error) => report.error("arguments.hex", &error)?,
    }
    for (label, path, maximum) in [
        ("source.ck", request.source, limits.file_bytes),
        ("workload.cktune.toml", request.manifest, limits.file_bytes),
        ("runner.bin", request.runner, limits.file_bytes),
        ("compiler.bin", request.compiler, limits.compiler_bytes),
    ] {
        writeln!(
            report.file,
            "source\t{label}\t{}",
            hex(path.as_os_str().as_bytes())
        )?;
        let copied = copy_path(path, &destination, label, maximum, &mut report.remaining);
        report.copied(label, &copied)?;
    }
    if let Err(error) = capture_cache(&request, limits, &destination, &mut report) {
        report.error("compile-directory", &error)?;
    }
    writeln!(report.file, "captureComplete={}", report.complete)?;
    report.file.sync_all()?;
    if report.complete {
        Ok(())
    } else {
        Err(io::Error::other(
            "original failure capture incomplete; partial evidence retained",
        ))
    }
}

pub(super) fn native_identity_context() -> Result<Vec<u8>, String> {
    use calckernel::{KirConsumer, NativeCpu, NativeTarget};
    // These are post-exit observations, not an attestation from the failed
    // process. Independent physical-key matching is required before use.
    std::panic::catch_unwind(|| {
        let target = NativeTarget::host_with_cpu(NativeCpu::Native).map_err(|error| error.to_string())?;
        let triple = target.triple().map_err(|error| error.to_string())?;
        let cpu = target.cpu().map_err(|error| error.to_string())?;
        let raw_features = target.features().map_err(|error| error.to_string())?;
        let mut features = raw_features.split(',').filter(|feature| !feature.is_empty()).collect::<Vec<_>>();
        features.sort();
        features.dedup();
        let profile = target.kir_profile(KirConsumer::NativeExecutable).map_err(|error| error.to_string())?;
        Ok(format!(
            "CKTUNE-FAILURE-IDENTITY/1\nobservation=after-original-process-exit\nfailedPlanIdentityVerified=false\nversionHex={}\nllvmManifestSha256={}\nllvmBridgeAbi={}\ntripleHex={}\ncpuHex={}\nfeaturesHex={}\ntargetProfileDigest={}\n",
            hex(env!("CARGO_PKG_VERSION").as_bytes()),
            env!("CKC_LLVM_MANIFEST_SHA256"),
            calckernel::LLVM_BRIDGE_ABI_VERSION,
            hex(triple.as_bytes()),
            hex(cpu.as_bytes()),
            hex(features.join(",").as_bytes()),
            profile.digest_hex(),
        ).into_bytes())
    })
    .map_err(|_| "native identity observation panicked; original failure unchanged".to_string())?
}

fn arguments_hex(arguments: &[OsString], maximum: u64) -> io::Result<Vec<u8>> {
    let mut output = Vec::new();
    for (index, argument) in arguments.iter().enumerate() {
        let bytes = argument.as_os_str().as_bytes();
        let length = bytes
            .len()
            .checked_mul(2)
            .and_then(|size| size.checked_add(32));
        if length.is_none_or(|size| size as u64 > maximum.saturating_sub(output.len() as u64)) {
            return Err(io::Error::other("capture arguments exceed byte bound"));
        }
        writeln!(output, "{index}\t{}", hex(bytes))?;
    }
    Ok(output)
}

fn copy_path(
    path: &Path,
    destination: &Directory,
    label: &str,
    maximum: u64,
    remaining: &mut u64,
) -> io::Result<Copied> {
    let parent = Directory::open(
        path.parent()
            .ok_or_else(|| io::Error::other("missing parent"))?,
    )?;
    let name = path
        .file_name()
        .ok_or_else(|| io::Error::other("missing filename"))?;
    Source::open(&parent, name)?.copy_to(destination, OsStr::new(label), maximum, remaining, false)
}

struct Report {
    file: File,
    complete: bool,
    remaining: u64,
}

impl Report {
    fn error(&mut self, label: &str, error: &io::Error) -> io::Result<()> {
        self.complete = false;
        writeln!(
            self.file,
            "error\t{label}\t{}",
            hex(error.to_string().as_bytes())
        )
    }

    fn memory(
        &mut self,
        destination: &Directory,
        label: &str,
        bytes: &[u8],
        maximum: u64,
    ) -> io::Result<()> {
        let result = (|| {
            let size = bytes.len() as u64;
            if size > maximum || size > self.remaining {
                return Err(io::Error::other(
                    "capture file or total byte bound exceeded",
                ));
            }
            let mut file = destination.create_file(OsStr::new(label))?;
            self.remaining -= size;
            file.write_all(bytes)?;
            file.sync_all()?;
            writeln!(
                self.file,
                "memory\t{label}\t{size}\t{}",
                hex(&Sha256::digest(bytes))
            )
        })();
        if let Err(error) = result {
            self.error(label, &error)?;
        }
        Ok(())
    }

    fn copied(&mut self, label: &str, result: &io::Result<Copied>) -> io::Result<()> {
        match result {
            Ok(copied) => writeln!(
                self.file,
                "copied\t{label}\t{}\t{}\t{:?}",
                copied.identity.size, copied.sha256, copied.identity
            ),
            Err(error) => self.error(label, error),
        }
    }
}

fn capture_cache(
    request: &Request<'_>,
    limits: Limits,
    destination: &Directory,
    report: &mut Report,
) -> io::Result<()> {
    writeln!(
        report.file,
        "source\tcompile-directory\t{}",
        hex(request.compile_cache.as_os_str().as_bytes())
    )?;
    let source = Directory::open(request.compile_cache)?;
    let original_identity = source.identity()?;
    let cache_destination = destination.create_directory("compile", false)?;
    let (names, exceeded) = source.names(limits.cache_files)?;
    if exceeded {
        report.error(
            "compile-directory",
            &io::Error::other("capture cache entry count bound exceeded"),
        )?;
    }
    for name in names {
        let raw_name = name.as_os_str().as_bytes();
        if raw_name.len() != 64
            || !raw_name
                .iter()
                .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
        {
            report.error("invalid-cache-name", &io::Error::other(hex(raw_name)))?;
            continue;
        }
        let label = format!("compile/{}", String::from_utf8_lossy(raw_name));
        writeln!(report.file, "entry\t{label}")?;
        let result = Source::open(&source, &name).and_then(|file| {
            file.copy_to(
                &cache_destination,
                &name,
                limits.file_bytes,
                &mut report.remaining,
                true,
            )
        });
        report.copied(&label, &result)?;
        if let Ok(copied) = result {
            match verify_envelope(raw_name, &copied.bytes) {
                Ok(()) => writeln!(report.file, "envelope-valid\t{label}")?,
                Err(error) => {
                    writeln!(report.file, "envelope-invalid\t{label}")?;
                    report.error(&label, &error)?;
                }
            }
        }
    }
    if source.identity()? != original_identity {
        return Err(io::Error::other("compile directory changed during capture"));
    }
    Ok(())
}

// This checks the retained envelope independently, never through the cache API
// which can refresh mtimes or evict malformed originals. Payload identity and
// failed-plan association require the separate, read-only evidence analyzer.
fn verify_envelope(name: &[u8], bytes: &[u8]) -> io::Result<()> {
    if bytes.len() < 85 || &bytes[..8] != b"CKTCACH1" {
        return Err(io::Error::other("invalid envelope header"));
    }
    if bytes[8..12] != 1_u32.to_be_bytes() || bytes[12] != 1 {
        return Err(io::Error::other("envelope schema/domain mismatch"));
    }
    if hex(&bytes[13..45]).as_bytes() != name {
        return Err(io::Error::other("envelope physical key/name mismatch"));
    }
    let length = u64::from_be_bytes(
        bytes[45..53]
            .try_into()
            .map_err(|_| io::Error::other("truncated length"))?,
    );
    if length != (bytes.len() - 85) as u64 {
        return Err(io::Error::other("envelope length mismatch"));
    }
    let end = bytes.len() - 32;
    let mut hash = Sha256::new();
    hash.update(b"CK-TUNE-CACHE-ENTRY\0");
    hash.update(&bytes[..end]);
    if hash.finalize().as_slice() != &bytes[end..] {
        return Err(io::Error::other("envelope checksum mismatch"));
    }
    Ok(())
}
