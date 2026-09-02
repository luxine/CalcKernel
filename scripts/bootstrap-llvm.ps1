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
$repoRoot = Split-Path -Parent $PSScriptRoot
. (Join-Path $PSScriptRoot "validate-msvc-crt.ps1")

function Import-MsvcEnvironment([string]$RequestedTarget, [string]$ProbeRoot) {
    $programFilesX86 = ${env:ProgramFiles(x86)}
    if ([string]::IsNullOrWhiteSpace($programFilesX86)) {
        $programFilesX86 = $env:ProgramFiles
    }
    $vswhere = Join-Path $programFilesX86 "Microsoft Visual Studio/Installer/vswhere.exe"
    if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
        throw "Visual Studio locator does not exist: $vswhere"
    }

    $requiredComponents = @("Microsoft.VisualStudio.Component.VC.Tools.x86.x64")
    if ($RequestedTarget.StartsWith("aarch64")) {
        $requiredComponents += "Microsoft.VisualStudio.Component.VC.Tools.ARM64"
    }
    $installations = @(
        & $vswhere -latest -products "*" -requires $requiredComponents -property installationPath
    )
    if ($LASTEXITCODE -ne 0) { throw "vswhere.exe failed to locate Visual Studio" }
    $installationPath = $installations |
        Where-Object { -not [string]::IsNullOrWhiteSpace($_) } |
        Select-Object -First 1
    if ([string]::IsNullOrWhiteSpace($installationPath)) {
        throw "Visual Studio with MSVC is not installed"
    }

    $vsDevCmd = Join-Path $installationPath "Common7/Tools/VsDevCmd.bat"
    if (-not (Test-Path -LiteralPath $vsDevCmd -PathType Leaf)) {
        throw "Visual Studio developer command does not exist: $vsDevCmd"
    }
    $msvcTargetArch = if ($RequestedTarget.StartsWith("aarch64")) { "arm64" } else { "amd64" }
    $msvcHostArch = "amd64"
    $devCommand = "call `"$vsDevCmd`" -no_logo -arch=$msvcTargetArch -host_arch=$msvcHostArch >nul && set"
    $environmentLines = @(& $env:ComSpec /d /s /c $devCommand)
    if ($LASTEXITCODE -ne 0) {
        throw "VsDevCmd.bat failed for target $msvcTargetArch on host tools $msvcHostArch"
    }
    foreach ($line in $environmentLines) {
        if ($line -match '^([^=]+)=(.*)$') {
            [Environment]::SetEnvironmentVariable($Matches[1], $Matches[2], "Process")
        }
    }

    if ([string]::IsNullOrWhiteSpace($env:VSCMD_ARG_TGT_ARCH)) {
        throw "VsDevCmd.bat did not record VSCMD_ARG_TGT_ARCH"
    }
    $null = Get-Command cl.exe -CommandType Application -ErrorAction Stop
    $null = Get-Command link.exe -CommandType Application -ErrorAction Stop

    $probe = Join-Path $ProbeRoot "ckc-msvc-target.c"
    $probeSource = @"
#if defined(_M_ARM64)
CKC_MSVC_TARGET=arm64
#elif defined(_M_X64)
CKC_MSVC_TARGET=x64
#else
CKC_MSVC_TARGET=unsupported
#endif
"@
    Set-Content -LiteralPath $probe -Value $probeSource -Encoding ascii
    $probeOutput = (& cl.exe /nologo /EP /TC $probe 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0) { throw "MSVC target probe failed" }
    $expectedTarget = if ($RequestedTarget.StartsWith("aarch64")) {
        "CKC_MSVC_TARGET=arm64"
    } else {
        "CKC_MSVC_TARGET=x64"
    }
    if (-not $probeOutput.Contains($expectedTarget)) {
        throw "MSVC target mismatch: expected $expectedTarget, got $probeOutput"
    }
    Remove-Item -LiteralPath $probe -Force
}

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
Import-MsvcEnvironment -RequestedTarget $Target -ProbeRoot $binaryDir
tar -xf $Archive --strip-components=1 -C $sourceDir
if ($LASTEXITCODE -ne 0) { throw "failed to extract LLVM source" }

$configure = @(
    "-S", (Join-Path $sourceDir "llvm"), "-B", $binaryDir, "-G", "Ninja",
    "-DCMAKE_C_COMPILER=cl.exe", "-DCMAKE_CXX_COMPILER=cl.exe",
    "-DCMAKE_BUILD_TYPE=Release", "-DCMAKE_INSTALL_PREFIX=$Prefix",
    "-DLLVM_ENABLE_PROJECTS=$projects", "-DLLVM_TARGETS_TO_BUILD=$llvmTarget",
    "-DLLVM_ENABLE_ASSERTIONS=ON", "-DBUILD_SHARED_LIBS=OFF",
    "-DLLVM_BUILD_LLVM_DYLIB=OFF", "-DLLVM_LINK_LLVM_DYLIB=OFF",
    "-DLLVM_BUILD_LLVM_C_DYLIB=OFF",
    "-DLLVM_ENABLE_RTTI=OFF", "-DLLVM_ENABLE_EH=OFF",
    "-DLLVM_ENABLE_ZLIB=OFF", "-DLLVM_ENABLE_ZSTD=OFF",
    "-DLLVM_ENABLE_LIBXML2=OFF", "-DLLVM_ENABLE_TERMINFO=OFF",
    "-DLLVM_ENABLE_LIBEDIT=OFF", "-DLLVM_INCLUDE_TESTS=OFF",
    "-DLLVM_INCLUDE_BENCHMARKS=OFF", "-DLLVM_INCLUDE_EXAMPLES=OFF",
    "-DCMAKE_MSVC_RUNTIME_LIBRARY=MultiThreaded",
    "-DCMAKE_EXPORT_COMPILE_COMMANDS=ON"
)
if ($Profile -eq "oracle") {
    $configure += @(
        "-DLLVM_ENABLE_RUNTIMES=compiler-rt",
        "-DCOMPILER_RT_BUILD_BUILTINS=OFF",
        "-DCOMPILER_RT_BUILD_SANITIZERS=OFF",
        "-DCOMPILER_RT_BUILD_XRAY=OFF",
        "-DCOMPILER_RT_BUILD_LIBFUZZER=OFF",
        "-DCOMPILER_RT_BUILD_MEMPROF=OFF",
        "-DCOMPILER_RT_BUILD_ORC=OFF",
        "-DCOMPILER_RT_BUILD_PROFILE=ON"
    )
}
& cmake @configure
if ($LASTEXITCODE -ne 0) { throw "LLVM CMake configuration failed" }
Assert-MsvcCompileCommands -Path (Join-Path $binaryDir "compile_commands.json")
foreach ($language in @("C", "CXX")) {
    $metadata = Get-ChildItem -LiteralPath $binaryDir -Recurse -File |
        Where-Object { $_.Name -eq "CMake${language}Compiler.cmake" } |
        Select-Object -First 1
    if ($null -eq $metadata) {
        throw "LLVM CMake configuration omitted ${language} compiler metadata"
    }
    $identity = Get-Content -Raw -LiteralPath $metadata.FullName
    $identityVariable = if ($language -eq "C") {
        "CMAKE_C_COMPILER_ID"
    } else {
        "CMAKE_CXX_COMPILER_ID"
    }
    $expectedIdentity = "set(${identityVariable} `"MSVC`")"
    if (-not $identity.Contains($expectedIdentity)) {
        throw "LLVM CMake selected a non-MSVC ${language} compiler"
    }
}

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
foreach ($directory in @("bin", "lib")) {
    if (Get-ChildItem -LiteralPath (Join-Path $Prefix $directory) -Filter "LLVM*.dll" -File) {
        throw "release prefix contains a shared LLVM library"
    }
}
$clang = Join-Path $Prefix "bin/clang.exe"
if ($Profile -eq "release" -and (Test-Path -LiteralPath $clang)) {
    throw "release prefix unexpectedly contains Clang"
}
if ($Profile -eq "oracle" -and -not (Test-Path -LiteralPath $clang -PathType Leaf)) {
    throw "oracle prefix is missing Clang"
}
if ($Profile -eq "oracle") {
    $profdata = Join-Path $Prefix "bin/llvm-profdata.exe"
    if (-not (Test-Path -LiteralPath $profdata -PathType Leaf)) {
        throw "oracle prefix is missing llvm-profdata"
    }
    $profileRuntime = Get-ChildItem -LiteralPath (Join-Path $Prefix "lib/clang") -Recurse -File |
        Where-Object { $_.Name -like "clang_rt.profile*.lib" -or $_.Name -like "libclang_rt.profile*.a" } |
        Select-Object -First 1
    if ($null -eq $profileRuntime) {
        throw "oracle prefix is missing the pinned compiler-rt profile runtime"
    }
}

$components = @("core", "native", "orcjit", "nativecodegen", "lto")
# LLVM 22 COFF also calls LibDriver and WindowsManifest outside the core/ORC/LTO closure.
$linkComponents = $components + @("libdriver", "windowsmanifest")
$libraryOutput = & $llvmConfig --link-static --libnames @linkComponents
if ($LASTEXITCODE -ne 0) { throw "llvm-config static library query failed" }
$llvmLibraries = ($libraryOutput -split "\s+") |
    Where-Object { $_ -ne "" } |
    ForEach-Object { $_ -replace '^lib', '' -replace '\.lib$', '' }
$dtltoArchive = Join-Path $Prefix "lib/LLVMDTLTO.lib"
if (-not (Test-Path -LiteralPath $dtltoArchive -PathType Leaf)) {
    throw "LLVM 22 static install is missing LLVMDTLTO.lib"
}
$staticLibraries = @("lldCOFF", "lldCommon", "LLVMDTLTO") + $llvmLibraries
$archives = @($staticLibraries | ForEach-Object { Join-Path $Prefix "lib/$_.lib" })
Assert-MsvcStaticArchives -ReadObj (Join-Path $Prefix "bin/llvm-readobj.exe") -Archives $archives
$systemOutput = & $llvmConfig --link-static --system-libs @linkComponents
if ($LASTEXITCODE -ne 0) { throw "llvm-config system library query failed" }
$systemLibraries = ($systemOutput -split "\s+") |
    Where-Object { $_ -ne "" } |
    ForEach-Object { $_ -replace '^[-/]DEFAULTLIB:', '' -replace '\.lib$', '' }

$runtimeDir = Join-Path $Prefix "share/ckc/runtime"
New-Item -ItemType Directory -Path $runtimeDir -Force | Out-Null
$runtimeInclude = Join-Path $repoRoot "native/runtime/include"
$runtimeVendor = Join-Path $repoRoot "native/runtime/vendor"
$runtimeSources = @(
    @("runtime.obj", (Join-Path $repoRoot "native/runtime/common/runtime.c")),
    @("format_int.obj", (Join-Path $repoRoot "native/runtime/common/format_int.c")),
    @("format_float.obj", (Join-Path $repoRoot "native/runtime/common/format_float.c")),
    @("ryu.obj", (Join-Path $repoRoot "native/runtime/vendor/ryu/d2s.c")),
    @("platform.obj", (Join-Path $repoRoot "native/runtime/windows/process.c"))
)
$runtimeObjects = @()
foreach ($item in $runtimeSources) {
    $name = $item[0]
    $source = $item[1]
    $destination = Join-Path $runtimeDir $name
    & cl.exe /nologo /c /TC /O2 /W3 /WX /GS- /Zl /Gy /Gw /DNDEBUG /DCKC_RYU_NO_MALLOC=1 "/I$runtimeInclude" "/I$runtimeVendor" "/Fo$destination" $source
    if ($LASTEXITCODE -ne 0) { throw "native runtime compilation failed: $source" }
    $runtimeObjects += $name
}
$runtimeHashes = $runtimeObjects | ForEach-Object {
    (Get-FileHash -LiteralPath (Join-Path $runtimeDir $_) -Algorithm SHA256).Hash.ToLowerInvariant()
}
$profileRuntimeObject = "profile_runtime.obj"
$profileRuntimeSource = Join-Path $repoRoot "native/profile_runtime/profile_runtime.c"
$profileRuntimePath = Join-Path $runtimeDir $profileRuntimeObject
$profileRuntimeInclude = Join-Path $repoRoot "native/profile_runtime/include"
$profileRuntimeRoot = Join-Path $repoRoot "native/profile_runtime"
& cl.exe /nologo /c /TC /std:c11 /O2 /W3 /WX /GS- /Zl /Gy /Gw /DNDEBUG "/I$profileRuntimeInclude" "/I$profileRuntimeRoot" "/Fo$profileRuntimePath" $profileRuntimeSource
if ($LASTEXITCODE -ne 0) { throw "profile runtime compilation failed: $profileRuntimeSource" }
$profileRuntimeHash = (Get-FileHash -LiteralPath $profileRuntimePath -Algorithm SHA256).Hash.ToLowerInvariant()
$dispatchRuntimeObject = "dispatch_runtime.obj"
$dispatchRuntimeSource = Join-Path $repoRoot "native/dispatch_runtime/dispatch_runtime.c"
$dispatchRuntimePath = Join-Path $runtimeDir $dispatchRuntimeObject
$dispatchRuntimeInclude = Join-Path $repoRoot "native/dispatch_runtime/include"
& cl.exe /nologo /c /TC /std:c11 /O2 /W3 /WX /GS- /Zl /Gy /Gw /DNDEBUG "/I$dispatchRuntimeInclude" "/Fo$dispatchRuntimePath" $dispatchRuntimeSource
if ($LASTEXITCODE -ne 0) { throw "dispatch runtime compilation failed: $dispatchRuntimeSource" }
$dispatchRuntimeHash = (Get-FileHash -LiteralPath $dispatchRuntimePath -Algorithm SHA256).Hash.ToLowerInvariant()
$runtimeJitSupport = $null
$runtimeJitSupportHash = $null
if ($Target -ceq "x86_64-pc-windows-msvc") {
    $runtimeJitSupport = "jit_image_base.obj"
    $runtimeJitSupportSource = Join-Path $repoRoot "native/runtime/windows/jit_image_base.c"
    $runtimeJitSupportPath = Join-Path $runtimeDir $runtimeJitSupport
    & cl.exe /nologo /c /TC /O2 /W3 /WX /GS- /Zl /Gy /Gw /DNDEBUG "/Fo$runtimeJitSupportPath" $runtimeJitSupportSource
    if ($LASTEXITCODE -ne 0) { throw "native JIT support compilation failed: $runtimeJitSupportSource" }
    $runtimeJitSupportHash = (Get-FileHash -LiteralPath $runtimeJitSupportPath -Algorithm SHA256).Hash.ToLowerInvariant()
}
$runtimeImport = "kernel32.lib"
$runtimeImportPath = Join-Path $runtimeDir $runtimeImport
$llvmLib = Join-Path $Prefix "bin/llvm-lib.exe"
if (-not (Test-Path -LiteralPath $llvmLib -PathType Leaf)) {
    throw "bootstrap did not install llvm-lib.exe for runtime import metadata"
}
$machine = if ($Target.StartsWith("aarch64")) { "arm64" } else { "x64" }
$definition = Join-Path $repoRoot "native/runtime/platform/kernel32.def"
& $llvmLib "/def:$definition" "/machine:$machine" "/out:$runtimeImportPath"
if ($LASTEXITCODE -ne 0 -or -not (Test-Path -LiteralPath $runtimeImportPath -PathType Leaf)) {
    throw "native runtime import-library generation failed"
}
$runtimeImportHash = (Get-FileHash -LiteralPath $runtimeImportPath -Algorithm SHA256).Hash.ToLowerInvariant()

$manifestDir = Join-Path $Prefix "share/ckc"
New-Item -ItemType Directory -Path $manifestDir -Force | Out-Null
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
    "msvc_runtime_library = `"MultiThreaded`"",
    "components = $(Format-TomlArray $components)",
    "static_libraries = $(Format-TomlArray $staticLibraries)",
    "system_libraries = $(Format-TomlArray $systemLibraries)",
    "runtime_objects = $(Format-TomlArray $runtimeObjects)",
    "runtime_sha256 = $(Format-TomlArray $runtimeHashes)",
    "profile_runtime_schema = 1",
    "profile_runtime_object = `"$profileRuntimeObject`"",
    "profile_runtime_sha256 = `"$profileRuntimeHash`"",
    "dispatch_runtime_schema = 1",
    "dispatch_runtime_object = `"$dispatchRuntimeObject`"",
    "dispatch_runtime_sha256 = `"$dispatchRuntimeHash`"",
    "runtime_platform_import = `"$runtimeImport`"",
    "runtime_platform_import_sha256 = `"$runtimeImportHash`""
)
if ($null -ne $runtimeJitSupport) {
    $manifest += @(
        "runtime_jit_support = `"$runtimeJitSupport`"",
        "runtime_jit_support_sha256 = `"$runtimeJitSupportHash`""
    )
}
Set-Content -LiteralPath (Join-Path $manifestDir "llvm-build.toml") -Value ($manifest -join "`n") -Encoding utf8NoBOM

if ($Profile -eq "oracle") { Write-Output $clang } else { Write-Output $Prefix }
