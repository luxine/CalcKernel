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

$prefix = $env:CKC_LLVM_PREFIX
if ([string]::IsNullOrWhiteSpace($prefix)) {
    throw "native artifact audit: CKC_LLVM_PREFIX is required"
}
$llvmReadobj = Join-Path $prefix "bin/llvm-readobj.exe"
if (-not (Test-Path -LiteralPath $llvmReadobj -PathType Leaf)) {
    throw "native artifact audit: missing pinned llvm-readobj"
}

function Invoke-CoffInspector {
    param([string]$Mode, [string]$Candidate)
    $output = (& $llvmReadobj $Mode $Candidate) -join "`n"
    if ($LASTEXITCODE -ne 0) {
        throw "native artifact audit: llvm-readobj $Mode failed"
    }
    return $output
}

function Get-CoffScopeNames {
    param([string]$Text, [string]$ScopePattern, [string]$Description)
    $names = @()
    $scopeDepth = 0
    $scopeName = $null
    foreach ($line in @($Text -split '\r?\n')) {
        if ($scopeDepth -eq 0) {
            if ($line -match "^(?:$ScopePattern)[ \t]*\{[ \t]*$") {
                $scopeDepth = 1
                $scopeName = $null
            }
            continue
        }
        if ($scopeDepth -eq 1 -and
            $line -match '^[ \t]+Name:[ \t]*(?<name>[^\r\n]+?)[ \t]*$') {
            if ($null -ne $scopeName) {
                throw "native artifact audit: malformed $Description"
            }
            $scopeName = $Matches['name'].Trim()
        }
        $scopeDepth += [regex]::Matches($line, '\{').Count
        $scopeDepth -= [regex]::Matches($line, '\}').Count
        if ($scopeDepth -lt 0) {
            throw "native artifact audit: malformed $Description"
        }
        if ($scopeDepth -eq 0) {
            if ([string]::IsNullOrWhiteSpace($scopeName)) {
                throw "native artifact audit: malformed $Description"
            }
            $names += $scopeName
            $scopeName = $null
        }
    }
    if ($scopeDepth -ne 0) {
        throw "native artifact audit: malformed $Description"
    }
    return $names
}

function Get-CoffSymbolNames {
    param([string]$Text)
    $names = @()
    $seenTable = $false
    $inTable = $false
    $symbolDepth = 0
    $symbolName = $null
    foreach ($line in @($Text -split '\r?\n')) {
        if (-not $inTable) {
            if ($line -match '^Symbols[ \t]*\[[ \t]*$') {
                if ($seenTable) {
                    throw "native artifact audit: malformed symbol table"
                }
                $seenTable = $true
                $inTable = $true
            }
            continue
        }
        if ($symbolDepth -eq 0) {
            if ($line -match '^[ \t]+Symbol[ \t]*\{[ \t]*$') {
                $symbolDepth = 1
                $symbolName = $null
                continue
            }
            if ($line -match '^\][ \t]*$') {
                $inTable = $false
                continue
            }
            if (-not [string]::IsNullOrWhiteSpace($line)) {
                throw "native artifact audit: malformed symbol table"
            }
            continue
        }
        if ($symbolDepth -eq 1 -and
            $line -match '^[ \t]+Name:[ \t]*(?<name>[^\r\n]+?)[ \t]*$') {
            if ($null -ne $symbolName) {
                throw "native artifact audit: malformed symbol table"
            }
            $symbolName = $Matches['name'].Trim()
        }
        $symbolDepth += [regex]::Matches($line, '\{').Count
        $symbolDepth -= [regex]::Matches($line, '\}').Count
        if ($symbolDepth -lt 0) {
            throw "native artifact audit: malformed symbol table"
        }
        if ($symbolDepth -eq 0) {
            if ([string]::IsNullOrWhiteSpace($symbolName)) {
                throw "native artifact audit: malformed symbol table"
            }
            $names += $symbolName
            $symbolName = $null
        }
    }
    if (-not $seenTable -or $inTable -or $symbolDepth -ne 0) {
        throw "native artifact audit: malformed symbol table"
    }
    return $names
}

$programImports = Invoke-CoffInspector --coff-imports (Join-Path $root "program.exe")
$dependencyNames = @(Get-CoffScopeNames $programImports 'Import|DelayImport' 'import descriptor')
if (@($dependencyNames | Where-Object { $_ -notmatch '(?i)^[a-z0-9][a-z0-9._-]*\.dll$' }).Count -ne 0) {
    throw "native artifact audit: malformed import descriptor name"
}
$dependencyNames = @($dependencyNames | ForEach-Object { $_.ToLowerInvariant() } | Sort-Object -Unique)
if ($dependencyNames.Count -ne 1 -or $dependencyNames[0] -ne "kernel32.dll") {
    throw "native artifact audit: executable dependencies must be exactly kernel32.dll"
}
$moduleImports = Invoke-CoffInspector --coff-imports (Join-Path $root "module.dll")
$dllDependencies = @(Get-CoffScopeNames $moduleImports 'Import|DelayImport' 'import descriptor')
if ($dllDependencies.Count -ne 0) {
    throw "native artifact audit: computation DLL must have no imports"
}
$moduleExports = Invoke-CoffInspector --coff-exports (Join-Path $root "module.dll")
$exports = @(Get-CoffScopeNames $moduleExports 'Export' 'export descriptor')
if (@($exports | Where-Object { $_ -notmatch '^[A-Za-z_?@$][A-Za-z0-9_?@$.-]*$' }).Count -ne 0) {
    throw "native artifact audit: malformed export descriptor name"
}
if ($exports -notcontains "answer") {
    throw "native artifact audit: computation DLL does not export answer"
}
$forbiddenExports = @($exports | Where-Object { $_ -match '(?i)LLVM|LLD|Clang|CalcKernel|__ck_' })
if ($forbiddenExports.Count -ne 0) {
    throw "native artifact audit: forbidden computation DLL export"
}
$forbidden = '(?i)\b(malloc|calloc|realloc|free|printf|fprintf|sprintf|snprintf|vsnprintf|setlocale|localeconv|__stack_chk_fail)\b'
foreach ($object in Get-ChildItem -LiteralPath $runtime -Filter "*.obj" -File) {
    $symbols = Invoke-CoffInspector --symbols $object.FullName
    $symbolNames = @(Get-CoffSymbolNames $symbols)
    if ($symbolNames.Count -eq 0) {
        throw "native artifact audit: llvm-readobj reported no symbol descriptors"
    }
    if (@($symbolNames | Where-Object { $_ -match $forbidden }).Count -ne 0) {
        throw "native artifact audit: forbidden runtime symbol in $($object.FullName)"
    }
}

Write-Output "native artifact audit passed: $root"
