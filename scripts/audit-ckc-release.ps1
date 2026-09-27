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
$importNames = @()
$descriptorDepth = 0
$descriptorName = $null
foreach ($line in @($imports -split '\r?\n')) {
    if ($descriptorDepth -eq 0) {
        if ($line -match '^(?:Import|DelayImport)[ \t]*\{[ \t]*$') {
            $descriptorDepth = 1
            $descriptorName = $null
        }
        continue
    }

    if ($descriptorDepth -eq 1 -and
        $line -match '^[ \t]+Name:[ \t]*(?<name>[^\r\n]+?)[ \t]*$') {
        if ($null -ne $descriptorName) {
            throw "ckc release audit: malformed import descriptor"
        }
        $descriptorName = $Matches['name'].Trim()
    }
    $descriptorDepth += [regex]::Matches($line, '\{').Count
    $descriptorDepth -= [regex]::Matches($line, '\}').Count
    if ($descriptorDepth -lt 0) {
        throw "ckc release audit: malformed import descriptor"
    }
    if ($descriptorDepth -eq 0) {
        if ([string]::IsNullOrWhiteSpace($descriptorName)) {
            throw "ckc release audit: malformed import descriptor"
        }
        $importNames += $descriptorName
        $descriptorName = $null
    }
}
if ($descriptorDepth -ne 0) {
    throw "ckc release audit: malformed import descriptor"
}
if ($importNames.Count -eq 0) {
    throw "ckc release audit: llvm-readobj reported no import descriptors"
}
$malformedNames = @($importNames | Where-Object { $_ -notmatch '(?i)^[a-z0-9][a-z0-9._-]*\.dll$' })
if ($malformedNames.Count -ne 0) {
    throw "ckc release audit: malformed import descriptor name"
}
$forbiddenPattern = '(?i)LLVM|LLD|Clang|CalcKernel|libck|MSVCP|VCRUNTIME|CONCRT|libstdc\+\+|libc\+\+'
$forbiddenNames = @($importNames | Where-Object { $_ -match $forbiddenPattern })
if ($forbiddenNames.Count -ne 0) {
    throw "ckc release audit: dynamic compiler or non-system C++ runtime dependency detected: $($forbiddenNames -join ', ')"
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
