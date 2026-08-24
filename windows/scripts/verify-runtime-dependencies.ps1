$ErrorActionPreference = "Stop"

$executable = [System.IO.Path]::GetFullPath(
    (Join-Path $PSScriptRoot "..\target\release\moqcast-windows.exe")
)
if (-not (Test-Path -LiteralPath $executable -PathType Leaf)) {
    throw "Release executable not found: $executable"
}

$vswhere = Join-Path ${env:ProgramFiles(x86)} "Microsoft Visual Studio\Installer\vswhere.exe"
if (-not (Test-Path -LiteralPath $vswhere -PathType Leaf)) {
    throw "Visual Studio locator not found: $vswhere"
}

$dumpbinCandidates = @(
    & $vswhere -latest -products * `
        -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 `
        -find "VC\Tools\MSVC\**\bin\Hostx64\x64\dumpbin.exe"
)
if ($LASTEXITCODE -ne 0 -or $dumpbinCandidates.Count -eq 0) {
    throw "dumpbin.exe was not found in the installed Visual Studio toolchain"
}
$dumpbin = $dumpbinCandidates[0]

$dependents = @(& $dumpbin /nologo /dependents $executable 2>&1)
$dumpbinExitCode = $LASTEXITCODE
$dependents | ForEach-Object { Write-Host $_ }
if ($dumpbinExitCode -ne 0) {
    throw "dumpbin.exe failed with exit code $dumpbinExitCode"
}

$dynamicRuntimePattern = '(?i)\b(?:vcruntime\d*[a-z0-9_]*|msvcp\d*[a-z0-9_]*|ucrtbase|api-ms-win-crt-[a-z0-9-]+)\.dll\b'
$dynamicRuntimes = @(
    $dependents |
        Select-String -Pattern $dynamicRuntimePattern -AllMatches |
        ForEach-Object { $_.Matches.Value } |
        Sort-Object -Unique
)
if ($dynamicRuntimes.Count -gt 0) {
    throw "Release executable dynamically depends on Microsoft VC/UCRT libraries: $($dynamicRuntimes -join ', ')"
}

Write-Host "Release executable has no dynamic Microsoft VC/UCRT dependency."
