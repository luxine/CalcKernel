param(
    [Parameter(Mandatory = $true)][string]$Archive,
    [Parameter(Mandatory = $true)][string]$Prefix,
    [Parameter(Mandatory = $true)][ValidateSet("aarch64-pc-windows-msvc", "x86_64-pc-windows-msvc")][string]$Target,
    [ValidateSet("release", "oracle")][string]$Profile = "release",
    [string]$BuildDir = "",
    [int]$Jobs = 0
)

$ErrorActionPreference = "Stop"
$llvmVersion = "22.1.8"
$llvmSha256 = "922f1817a0df7b1489272d18134ee0087a8b068828f87ac63b9861b1a9965888"

if (-not (Test-Path -LiteralPath $Archive -PathType Leaf)) {
    throw "LLVM source archive does not exist: $Archive"
}
if (Test-Path -LiteralPath $Prefix) {
    throw "refusing to overwrite LLVM prefix: $Prefix"
}
if ($BuildDir -eq "") {
    $BuildDir = Join-Path "build/llvm" "$Target-$Profile"
}
if (Test-Path -LiteralPath $BuildDir) {
    throw "refusing to overwrite LLVM build directory: $BuildDir"
}

$actualSha = (Get-FileHash -LiteralPath $Archive -Algorithm SHA256).Hash.ToLowerInvariant()
if ($actualSha -ne $llvmSha256) {
    throw "LLVM source checksum mismatch: expected $llvmSha256, got $actualSha"
}

$llvmTarget = if ($Target.StartsWith("aarch64")) { "AArch64" } else { "X86" }
$projects = if ($Profile -eq "oracle") { "clang;lld" } else { "lld" }
$sourceDir = Join-Path $BuildDir "source"
$binaryDir = Join-Path $BuildDir "build"
New-Item -ItemType Directory -Path $sourceDir, $binaryDir | Out-Null
tar -xf $Archive --strip-components=1 -C $sourceDir
if ($LASTEXITCODE -ne 0) { throw "failed to extract LLVM source" }

$configure = @(
    "-S", (Join-Path $sourceDir "llvm"), "-B", $binaryDir, "-G", "Ninja",
    "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_INSTALL_PREFIX=$Prefix",
    "-DLLVM_ENABLE_PROJECTS=$projects", "-DLLVM_TARGETS_TO_BUILD=$llvmTarget",
    "-DLLVM_ENABLE_ASSERTIONS=ON", "-DBUILD_SHARED_LIBS=OFF",
    "-DLLVM_BUILD_LLVM_DYLIB=OFF", "-DLLVM_LINK_LLVM_DYLIB=OFF",
    "-DLLVM_ENABLE_RTTI=OFF", "-DLLVM_ENABLE_EH=OFF",
    "-DLLVM_ENABLE_ZLIB=OFF", "-DLLVM_ENABLE_ZSTD=OFF",
    "-DLLVM_ENABLE_LIBXML2=OFF", "-DLLVM_ENABLE_TERMINFO=OFF",
    "-DLLVM_ENABLE_LIBEDIT=OFF", "-DLLVM_INCLUDE_TESTS=OFF",
    "-DLLVM_INCLUDE_BENCHMARKS=OFF", "-DLLVM_INCLUDE_EXAMPLES=OFF"
)
& cmake @configure
if ($LASTEXITCODE -ne 0) { throw "LLVM CMake configuration failed" }

$build = @("--build", $binaryDir)
if ($Jobs -gt 0) { $build += @("--parallel", "$Jobs") }
& cmake @build
if ($LASTEXITCODE -ne 0) { throw "LLVM build failed" }
& cmake --install $binaryDir
if ($LASTEXITCODE -ne 0) { throw "LLVM install failed" }

$llvmConfig = Join-Path $Prefix "bin/llvm-config.exe"
if (-not (Test-Path -LiteralPath $llvmConfig -PathType Leaf)) {
    throw "bootstrap did not install llvm-config.exe"
}
$installedVersion = (& $llvmConfig --version).Trim()
if ($installedVersion -ne $llvmVersion) { throw "installed llvm-config version mismatch" }
if (Get-ChildItem -LiteralPath (Join-Path $Prefix "lib") -Filter "LLVM*.dll" -File) {
    throw "release prefix contains a shared LLVM library"
}
$clang = Join-Path $Prefix "bin/clang.exe"
if ($Profile -eq "release" -and (Test-Path -LiteralPath $clang)) {
    throw "release prefix unexpectedly contains Clang"
}
if ($Profile -eq "oracle" -and -not (Test-Path -LiteralPath $clang -PathType Leaf)) {
    throw "oracle prefix is missing Clang"
}

$components = @("core", "native", "orcjit", "nativecodegen")
$llvmLibraries = ((& $llvmConfig --link-static --libnames @components) -split "\s+") |
    Where-Object { $_ -ne "" } |
    ForEach-Object { $_ -replace '^lib', '' -replace '\.lib$', '' }
$staticLibraries = @("lldCOFF", "lldCommon") + $llvmLibraries
$systemLibraries = ((& $llvmConfig --link-static --system-libs @components) -split "\s+") |
    Where-Object { $_ -ne "" } |
    ForEach-Object { $_ -replace '^[-/]DEFAULTLIB:', '' -replace '\.lib$', '' }

$manifestDir = Join-Path $Prefix "share/ckc"
New-Item -ItemType Directory -Path $manifestDir | Out-Null
function Format-TomlArray([string[]]$Values) {
    return "[" + (($Values | ForEach-Object { '"' + $_ + '"' }) -join ", ") + "]"
}
$manifest = @(
    "schema = 1",
    "version = `"$llvmVersion`"",
    "target = `"$Target`"",
    "profile = `"$Profile`"",
    "source_sha256 = `"$llvmSha256`"",
    "static_only = true",
    "components = $(Format-TomlArray $components)",
    "static_libraries = $(Format-TomlArray $staticLibraries)",
    "system_libraries = $(Format-TomlArray $systemLibraries)"
) -join "`n"
Set-Content -LiteralPath (Join-Path $manifestDir "llvm-build.toml") -Value $manifest -Encoding utf8NoBOM

if ($Profile -eq "oracle") { Write-Output $clang } else { Write-Output $Prefix }
