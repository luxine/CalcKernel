param(
    [Parameter(Mandatory = $true)][string]$Prefix,
    [Parameter(Mandatory = $true)][string]$Target,
    [Parameter(Mandatory = $true)][ValidateSet("release", "oracle")][string]$Profile
)
$ErrorActionPreference = "Stop"
Set-StrictMode -Version Latest

$manifestPath = Join-Path $Prefix "share/ckc/llvm-build.toml"
$manifestLines = @(Get-Content -LiteralPath $manifestPath)

# The producer emits a closed subset: one scalar or JSON-compatible string array
# per line. Reject duplicate keys and wrong types instead of substring matching.
function Read-Value([string]$Key) {
    $pattern = '^' + [regex]::Escape($Key) + '\s*=\s*(.+?)\s*$'
    $values = @($manifestLines | ForEach-Object {
        if ($_ -cmatch $pattern) { $Matches[1] }
    })
    if ($values.Count -ne 1) { throw "missing or duplicate manifest key: $Key" }
    return $values[0]
}
function Read-String([string]$Key) {
    $value = Read-Value $Key | ConvertFrom-Json -NoEnumerate
    if ($value -isnot [string]) { throw "manifest key is not a string: $Key" }
    return $value
}
function Read-Strings([string]$Key) {
    $value = Read-Value $Key | ConvertFrom-Json -NoEnumerate
    if ($value -isnot [array]) { throw "manifest key is not a string array: $Key" }
    foreach ($item in $value) {
        if ($item -isnot [string]) { throw "manifest array contains a non-string: $Key" }
        $item
    }
}
function Assert-Hash([string]$Path, [string]$Expected, [string]$Subject) {
    if ($Expected -cnotmatch '^[0-9a-f]{64}$') { throw "invalid $Subject hash" }
    $actual = (Get-FileHash -Algorithm SHA256 -LiteralPath $Path).Hash.ToLowerInvariant()
    if ($actual -cne $Expected) { throw "$Subject hash mismatch: $Path" }
}

if ((Read-Value "schema") -cne "1") { throw "unsupported LLVM manifest schema" }
if ((Read-String "version") -cne "22.1.8") { throw "unexpected LLVM manifest version" }
if ((Read-String "target") -cne $Target) { throw "LLVM manifest target mismatch" }
if ((Read-String "profile") -cne $Profile) { throw "LLVM manifest profile mismatch" }
if ((Read-String "source_sha256") -cne "922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888") {
    throw "LLVM source archive hash mismatch"
}
if ((Read-Value "static_only") -cne "true") { throw "LLVM prefix is not static-only" }
$isMsvc = $Target.EndsWith("-msvc", [StringComparison]::Ordinal)
$configName = if ($isMsvc) { "llvm-config.exe" } else { "llvm-config" }
$config = Join-Path $Prefix "bin/$configName"
if (-not (Test-Path -LiteralPath $config -PathType Leaf)) { throw "missing llvm-config" }
$version = & $config --version
if ($LASTEXITCODE -ne 0 -or "$version".Trim() -cne "22.1.8") { throw "unexpected llvm-config version" }

$libraries = @(Read-Strings "static_libraries")
if ($libraries -cnotcontains "LLVMDTLTO") { throw "missing static LLVMDTLTO component" }
foreach ($library in $libraries) {
    if ($library -cnotmatch '^[A-Za-z0-9_+-]+$') { throw "invalid static library name" }
    $archiveName = if ($isMsvc) { "$library.lib" } else { "lib$library.a" }
    if (-not (Test-Path -LiteralPath (Join-Path $Prefix "lib/$archiveName") -PathType Leaf)) {
        throw "missing static library: $archiveName"
    }
}
foreach ($directory in @("bin", "lib")) {
    foreach ($file in Get-ChildItem -LiteralPath (Join-Path $Prefix $directory) -File) {
        if ($file.Name -match '^(libLLVM.*\.(so(\..*)?|dylib)|LLVM.*\.dll)$') {
            throw "shared LLVM library in static prefix: $($file.Name)"
        }
    }
}

$objects = @(Read-Strings "runtime_objects")
$hashes = @(Read-Strings "runtime_sha256")
if ($objects.Count -ne 5 -or $hashes.Count -ne 5) { throw "runtime object/hash count mismatch" }
$suffix = if ($isMsvc) { ".obj" } else { ".o" }
$expectedNames = @("runtime", "format_int", "format_float", "ryu", "platform")
for ($index = 0; $index -lt 5; $index++) {
    if ($objects[$index] -cne ($expectedNames[$index] + $suffix)) { throw "invalid runtime object name" }
    Assert-Hash (Join-Path $Prefix "share/ckc/runtime/$($objects[$index])") $hashes[$index] "runtime object"
}
if ($isMsvc) {
    $import = Read-String "runtime_platform_import"
    if ($import -cnotmatch '^[A-Za-z0-9_-]+\.lib$') { throw "invalid runtime platform import name" }
    Assert-Hash (Join-Path $Prefix "share/ckc/runtime/$import") (Read-String "runtime_platform_import_sha256") "runtime import"
}
$clangName = if ($isMsvc) { "clang.exe" } else { "clang" }
$clang = Join-Path $Prefix "bin/$clangName"
if ($Profile -eq "release" -and (Test-Path -LiteralPath $clang)) { throw "release prefix contains Clang" }
if ($Profile -eq "oracle") {
    if (-not (Test-Path -LiteralPath $clang -PathType Leaf)) { throw "missing Clang oracle" }
    $clangVersion = & $clang --version
    if ($LASTEXITCODE -ne 0 -or "$clangVersion" -notmatch '\bclang version 22\.1\.8\b') { throw "unexpected Clang oracle version" }
}
Write-Output "LLVM prefix verified: $Target / $Profile"
