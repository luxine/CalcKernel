param(
    [Parameter(Mandatory = $true)][string]$Path
)

$ErrorActionPreference = "Stop"
$candidate = (Resolve-Path -LiteralPath $Path).Path
if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
    throw "ckc release audit: missing release executable"
}

$dumpbin = (Get-Command dumpbin.exe -ErrorAction Stop).Source
$imports = (& $dumpbin /nologo /dependents $candidate) -join "`n"
if ($LASTEXITCODE -ne 0) { throw "ckc release audit: dumpbin /dependents failed" }
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
