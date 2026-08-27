param(
    [Parameter(Mandatory = $true)][string]$Path
)

$ErrorActionPreference = "Stop"
$root = (Resolve-Path -LiteralPath $Path).Path
$runtime = Join-Path $root "runtime"
$required = @(
    "module.obj", "module-static.lib", "module.dll", "module-import.lib",
    "program.exe", "runtime/runtime.obj", "runtime/format_int.obj",
    "runtime/format_float.obj", "runtime/ryu.obj", "runtime/platform.obj",
    "runtime/kernel32.lib", "runtime/SHA256SUMS"
)
foreach ($relative in $required) {
    $candidate = Join-Path $root $relative
    if (-not (Test-Path -LiteralPath $candidate -PathType Leaf)) {
        throw "native artifact audit: missing $candidate"
    }
}

foreach ($line in Get-Content -LiteralPath (Join-Path $runtime "SHA256SUMS")) {
    if ($line -notmatch '^([0-9a-f]{64})  ([A-Za-z0-9_.-]+)$') {
        throw "native artifact audit: malformed SHA256SUMS line"
    }
    $actual = (Get-FileHash -LiteralPath (Join-Path $runtime $Matches[2]) -Algorithm SHA256).Hash.ToLowerInvariant()
    if ($actual -ne $Matches[1]) {
        throw "native artifact audit: runtime hash mismatch for $($Matches[2])"
    }
}

$dumpbin = (Get-Command dumpbin.exe -ErrorAction Stop).Source
$dependencies = (& $dumpbin /nologo /dependents (Join-Path $root "program.exe")) -join "`n"
if ($LASTEXITCODE -ne 0) { throw "native artifact audit: dumpbin /dependents failed" }
$dependencyNames = @([regex]::Matches($dependencies, '(?im)^\s+([A-Za-z0-9_.-]+\.dll)\s*$') | ForEach-Object { $_.Groups[1].Value.ToLowerInvariant() } | Sort-Object -Unique)
if ($dependencyNames.Count -ne 1 -or $dependencyNames[0] -ne "kernel32.dll") {
    throw "native artifact audit: executable dependencies must be exactly kernel32.dll"
}
$dllDependencies = (& $dumpbin /nologo /dependents (Join-Path $root "module.dll")) -join "`n"
if ($dllDependencies -match '(?im)^\s+[A-Za-z0-9_.-]+\.dll\s*$') {
    throw "native artifact audit: computation DLL must have no imports"
}
$exports = (& $dumpbin /nologo /exports (Join-Path $root "module.dll")) -join "`n"
if ($exports -notmatch '(?m)^\s+\d+\s+[0-9A-F]+\s+[0-9A-F]+\s+answer\s*$') {
    throw "native artifact audit: computation DLL does not export answer"
}
if ($exports -match '(?i)LLVM|LLD|Clang|CalcKernel|__ck_') {
    throw "native artifact audit: forbidden computation DLL export"
}
$forbidden = '(?i)\b(malloc|calloc|realloc|free|printf|fprintf|sprintf|snprintf|vsnprintf|setlocale|localeconv|__stack_chk_fail)\b'
foreach ($object in Get-ChildItem -LiteralPath $runtime -Filter "*.obj" -File) {
    $symbols = (& $dumpbin /nologo /symbols $object.FullName) -join "`n"
    if ($symbols -match $forbidden) {
        throw "native artifact audit: forbidden runtime symbol in $($object.FullName)"
    }
}

Write-Output "native artifact audit passed: $root"
