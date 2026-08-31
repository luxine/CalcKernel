param(
    [Parameter(Mandatory = $true)][string]$Path
)

$ErrorActionPreference = "Stop"
$candidate = (Resolve-Path -LiteralPath $Path).Path
if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
    throw "ckc release audit: missing release executable"
}

$prefix = $env:CKC_LLVM_PREFIX
if ([string]::IsNullOrWhiteSpace($prefix)) {
    throw "ckc release audit: CKC_LLVM_PREFIX is required"
}
$llvmReadobj = Join-Path $prefix "bin/llvm-readobj.exe"
if (-not (Test-Path -LiteralPath $llvmReadobj -PathType Leaf)) {
    throw "ckc release audit: missing pinned llvm-readobj"
}
$imports = (& $llvmReadobj --coff-imports $candidate) -join "`n"
if ($LASTEXITCODE -ne 0) { throw "ckc release audit: llvm-readobj --coff-imports failed" }
if ($imports -match '(?i)LLVM|LLD|Clang|CalcKernel|libck|MSVCP|VCRUNTIME|CONCRT|libstdc\+\+|libc\+\+') {
    throw "ckc release audit: dynamic compiler or non-system C++ runtime dependency detected"
}

$verbose = (& $candidate --version --verbose) -join "`n"
if ($LASTEXITCODE -ne 0 -or $verbose -notmatch '(?m)^LLVM: 22\.1\.8$') {
    throw "ckc release audit: verbose version evidence is missing LLVM 22.1.8"
}
$licenses = (& $candidate licenses) -join "`n"
if ($LASTEXITCODE -ne 0 -or $licenses -notmatch '(?m)^===== LLVM Project 22\.1\.8') {
    throw "ckc release audit: embedded LLVM notices are missing"
}

Write-Output "ckc release audit passed: $candidate"
