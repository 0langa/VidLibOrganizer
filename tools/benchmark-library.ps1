# Metadata-only benchmark: no large media library is created and no plan is applied.
[CmdletBinding()]
param(
    [ValidateRange(1, 100000)][int]$Entries = 100000,
    [string]$Fixture = (Join-Path ([IO.Path]::GetTempPath()) 'vidlib-metadata-100000.jsonl')
)
$ErrorActionPreference = 'Stop'
$oldEntries = $env:VIDLIB_BENCHMARK_ENTRIES
$oldFixture = $env:VIDLIB_BENCHMARK_FIXTURE
try {
    $env:VIDLIB_BENCHMARK_ENTRIES = [string]$Entries
    $env:VIDLIB_BENCHMARK_FIXTURE = [IO.Path]::GetFullPath($Fixture)
    Push-Location (Split-Path $PSScriptRoot -Parent)
    try {
        cargo test --locked -p vidlib-tauri --lib benchmark_metadata_library -- --ignored --nocapture
        if ($LASTEXITCODE -ne 0) { throw "Benchmark failed: $LASTEXITCODE" }
        Get-FileHash -Algorithm SHA256 $env:VIDLIB_BENCHMARK_FIXTURE
        cargo test --locked -p vidlib-tauri --lib benchmark_native_preview -- --ignored --nocapture
        if ($LASTEXITCODE -ne 0) { throw "Native preview benchmark failed: $LASTEXITCODE" }
    } finally { Pop-Location }
} finally {
    $env:VIDLIB_BENCHMARK_ENTRIES = $oldEntries
    $env:VIDLIB_BENCHMARK_FIXTURE = $oldFixture
}
