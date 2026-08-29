param(
    [Parameter(Mandatory = $true)][string]$Path
)

$ErrorActionPreference = "Stop"
$candidate = (Resolve-Path -LiteralPath $Path).Path
if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
    throw "JIT memory audit: missing ckc release executable"
}

$root = Join-Path ([IO.Path]::GetTempPath()) ("ckc-jit-audit-" + [Guid]::NewGuid().ToString("N"))
[IO.Directory]::CreateDirectory($root) | Out-Null
$source = Join-Path $root "program.ck"
$stdout = Join-Path $root "stdout.txt"
$stderr = Join-Path $root "stderr.txt"
$program = @"
fn main() -> i32 {
    print_i32(42);
    print_newline();
    return 0;
}
"@
[IO.File]::WriteAllText($source, $program, [Text.UTF8Encoding]::new($false))

$oldPath = $env:PATH
$oldAudit = $env:CKC_INTERNAL_JIT_AUDIT
try {
    $env:PATH = ""
    $env:CKC_INTERNAL_JIT_AUDIT = "1"
    & $candidate run $source --no-cache 1> $stdout 2> $stderr
    $status = $LASTEXITCODE
} finally {
    $env:PATH = $oldPath
    $env:CKC_INTERNAL_JIT_AUDIT = $oldAudit
}

try {
    if ($status -ne 0) {
        throw "JIT memory audit: ckc run failed: $(Get-Content -Raw -LiteralPath $stderr)"
    }
    if ((Get-Content -Raw -LiteralPath $stdout).TrimEnd("`r", "`n") -ne "42") {
        throw "JIT memory audit: program stdout mismatch"
    }
    $lines = @(Get-Content -LiteralPath $stderr)
    if ($lines.Count -ne 1 -or -not $lines[0].StartsWith("CKC_JIT_AUDIT_V1 ")) {
        throw "JIT memory audit: expected exactly one audit record"
    }
    $report = $lines[0]
    $required = @(
        ' layer=(jitlink|rtdyld-coff-aarch64)',
        ' allocations=[1-9][0-9]*',
        ' relocation=rw-nx',
        ' code=rx',
        ' data=nx',
        ' icache=flushed',
        ' icache-count=[1-9][0-9]*',
        ' map-jit=no',
        ' thread-wx-supported=no',
        ' thread-wx=no'
    )
    foreach ($field in $required) {
        if ($report -notmatch $field) {
            throw "JIT memory audit: missing policy evidence $field"
        }
    }
    if ([Runtime.InteropServices.RuntimeInformation]::OSArchitecture -eq
        [Runtime.InteropServices.Architecture]::Arm64 -and
        $report -notmatch ' layer=rtdyld-coff-aarch64') {
        throw "JIT memory audit: Windows AArch64 did not select RuntimeDyld"
    }
    Write-Output "JIT memory audit passed: $candidate"
} finally {
    Remove-Item -LiteralPath $root -Recurse -Force -ErrorAction SilentlyContinue
}
