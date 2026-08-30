# Shared by the producer and the cache boundary. Declarations alone do not prove CRT identity.
function Assert-MsvcCompileCommands([string]$Path) {
    $entries = Get-Content -Raw -LiteralPath $Path -ErrorAction Stop | ConvertFrom-Json -NoEnumerate
    if ($entries -isnot [array]) { throw "MSVC compile database must be an array" }
    $count = 0
    foreach ($entry in $entries) {
        if ($entry.file -notmatch '\.(c|cc|cpp|cxx|c\+\+)$') { continue }
        $count++
        $arguments = $entry.PSObject.Properties["arguments"]
        if ($null -ne $arguments) {
            $flags = @($arguments.Value | ForEach-Object {
                if ($_ -cmatch '^[-/](M[TD]d?)$') { $Matches[1] }
            })
        } else {
            $command = $entry.PSObject.Properties["command"]
            if ($null -eq $command -or $command.Value -isnot [string]) {
                throw "MSVC compile command missing for $($entry.file)"
            }
            $flags = @([regex]::Matches($command.Value, '(?:^|\s)"?[-/](M[TD]d?)"?(?=\s|$)') |
                ForEach-Object { $_.Groups[1].Value })
        }
        if ($flags.Count -eq 0 -or @($flags | Where-Object { $_ -cne "MT" }).Count -ne 0) {
            throw "MSVC compile command must use only release-static /MT: $($entry.file)"
        }
    }
    if ($count -eq 0) { throw "MSVC compile database has no C/C++ commands" }
    Write-Output "MSVC compile commands verified: $count C/C++ files use /MT"
}

function Assert-MsvcStaticArchives([string]$ReadObj, [string[]]$Archives) {
    if (-not (Test-Path -LiteralPath $ReadObj -PathType Leaf)) { throw "missing llvm-readobj: $ReadObj" }
    $version = (& $ReadObj --version 2>&1 | Out-String)
    if ($LASTEXITCODE -ne 0 -or $version -cnotmatch '\bLLVM version 22\.1\.8\b') {
        throw "unexpected llvm-readobj version: $version"
    }
    if ($Archives.Count -eq 0) { throw "no static archives to validate" }
    foreach ($archive in $Archives) {
        if (-not (Test-Path -LiteralPath $archive -PathType Leaf)) {
            throw "missing static archive: $archive"
        }
        # A separate invocation avoids the Windows command-line length limit.
        $lines = @(& $ReadObj --coff-directives $archive 2>&1)
        if ($LASTEXITCODE -ne 0) { throw "llvm-readobj failed for ${archive}: $lines" }
        $directives = @($lines | ForEach-Object {
            if ("$_" -cmatch '^\s*Directive\(s\):\s*(.*)$') { $Matches[1] }
        }) -join " "
        $staticEvidence = $false
        foreach ($item in [regex]::Matches($directives, '(?i)[/-]FAILIFMISMATCH:"?RuntimeLibrary=([^"\s]*)')) {
            if ($item.Groups[1].Value -cne "MT_StaticRelease") {
                throw "non-release-static CRT RuntimeLibrary in $archive"
            }
            $staticEvidence = $true
        }
        foreach ($item in [regex]::Matches($directives, '(?i)[/-]DEFAULTLIB:(?:"([^"]+)"|([^\s]+))')) {
            $library = if ($item.Groups[1].Success) { $item.Groups[1].Value } else { $item.Groups[2].Value }
            $library = (($library -split '[\\/]')[-1] -replace '(?i)\.lib$', '').ToLowerInvariant()
            if ($library -match '^(msvcrt[d]?|msvcprt[d]?|libcmtd|libcpmtd|vcruntime([0-9_]+)?d?|ucrt(d|base[d]?)?|msvcp[0-9_]+d?)$') {
                throw "non-release-static CRT DEFAULTLIB $library in $archive"
            }
            if ($library -in @("libcmt", "libcpmt")) { $staticEvidence = $true }
        }
        if (-not $staticEvidence) { throw "no release-static CRT evidence in $archive" }
    }
    Write-Output "MSVC archive CRT verified: $($Archives.Count) static libraries"
}
