[CmdletBinding(DefaultParameterSetName = 'Store')]
param(
    [Parameter(Mandatory, ParameterSetName = 'Development')][switch]$Development,
    [Parameter(Mandatory, ParameterSetName = 'Store')][string]$IdentityConfig,
    [ValidateSet('x64', 'arm64')][string]$Architecture = 'x64',
    [string]$Version,
    [string]$Python = 'python',
    [switch]$SkipBuild
)
$ErrorActionPreference = 'Stop'
$project = Split-Path $PSScriptRoot -Parent
if ($IdentityConfig) { $IdentityConfig = (Resolve-Path -LiteralPath $IdentityConfig).Path }
$target = if ($Architecture -eq 'arm64') { 'aarch64-pc-windows-msvc' } else { 'x86_64-pc-windows-msvc' }
$previousPath = $env:PATH
$previousFlags = $env:RUSTFLAGS
Push-Location $project
try {
    if ($env:CARGO_ENCODED_RUSTFLAGS) { throw 'Unset CARGO_ENCODED_RUSTFLAGS so the static CRT flag can take effect.' }
    # Process-local tool paths; no changes to the user's saved PATH.
    $candidates = @("$env:ProgramFiles\CMake\bin", "$env:ProgramFiles\NASM", "$env:ProgramFiles\Git\usr\bin")
    $candidates += @(Get-ChildItem "$project\ai\runtime\cmake-*\bin" -Directory -ErrorAction SilentlyContinue | ForEach-Object FullName)
    $candidates += @(Get-ChildItem "$project\ai\runtime\nasm-*" -Directory -ErrorAction SilentlyContinue | ForEach-Object FullName)
    $env:PATH = ($candidates -join ';') + ';' + $env:PATH
    $env:RUSTFLAGS = "$previousFlags -C target-feature=+crt-static".Trim()
    if (-not $SkipBuild) {
        foreach ($tool in @('cargo', 'rustc', 'cmake', 'perl', $Python)) { Get-Command $tool -ErrorAction Stop | Out-Null }
        if ($Architecture -eq 'x64') { Get-Command nasm -ErrorAction Stop | Out-Null }
        & cargo fetch --locked
        if ($LASTEXITCODE) { throw 'Cargo dependency download failed.' }
        $hostTarget = (& rustc -vV | Select-String '^host: (.+)$').Matches.Groups[1].Value
        $testArguments = @('test', '--locked', '--release', '--target', $target)
        if ($hostTarget -ne $target) {
            $testArguments += '--no-run'
            Write-Warning 'Cross compilation only: run these tests on a matching Windows CPU before release.'
        }
        & cargo @testArguments
        if ($LASTEXITCODE) { throw 'Windows tests failed.' }
        & cargo build --locked --release --bin nabi-view --target $target
        if ($LASTEXITCODE) { throw 'Windows build failed.' }
        & "$project/build/windows-previews.ps1" -Architecture $Architecture
        if ($LASTEXITCODE) { throw 'Windows previews failed.' }
        & $Python build/notices.py
        if ($LASTEXITCODE) { throw 'Third-party notice generation failed.' }
    }
    $arguments = @('build/package-msix.py', '--arch', $Architecture)
    if ($Development) { $arguments += '--development' } else { $arguments += @('--identity', $IdentityConfig) }
    if ($Version) { $arguments += @('--version', $Version) }
    & $Python @arguments
    if ($LASTEXITCODE) { throw 'MSIX packaging failed.' }
} finally {
    $env:PATH = $previousPath
    $env:RUSTFLAGS = $previousFlags
    Pop-Location
}
