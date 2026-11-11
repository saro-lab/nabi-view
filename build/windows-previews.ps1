[CmdletBinding()]
param(
    [ValidateSet('x64','arm64')][string]$Architecture='x64',
    [switch]$SkipRust,
    [switch]$Smoke
)
$ErrorActionPreference='Stop'
$project=Split-Path $PSScriptRoot -Parent
Push-Location $project
try {
    $target=if ($Architecture -eq 'arm64') {'aarch64-pc-windows-msvc'} else {'x86_64-pc-windows-msvc'}
    if (-not $SkipRust) {
        & cargo build --locked --release -p nabi-preview --target $target
        if ($LASTEXITCODE) { throw 'Rust preview decoder build failed.' }
    }
    $metadata=& cargo metadata --offline --locked --no-deps --format-version=1 | ConvertFrom-Json
    if ($LASTEXITCODE) { throw 'Cargo metadata failed.' }
    $output=Join-Path $metadata.target_directory "$target/release"
    $vswhere=Join-Path ${env:ProgramFiles(x86)} 'Microsoft Visual Studio/Installer/vswhere.exe'
    if (-not (Test-Path $vswhere)) { throw 'Visual Studio C++ Build Tools are required.' }
    $install=& $vswhere -latest -products '*' -requires Microsoft.VisualStudio.Component.VC.Tools.x86.x64 -property installationPath
    if (-not $install) { throw 'Visual Studio C++ tools were not found.' }
    $vcvars=Join-Path $install 'VC/Auxiliary/Build/vcvarsall.bat'
    $arch=if ($Architecture -eq 'arm64') {'x64_arm64'} else {'x64'}
    $script=Join-Path $output 'build-preview.cmd'
    @"
@echo off
call "$vcvars" $arch
if errorlevel 1 exit /b %errorlevel%
cl /nologo /std:c++17 /EHsc /W4 /MT /O2 /DUNICODE /D_UNICODE /LD "$project/platforms/preview/windows/preview.cpp" /Fo"$output/nabi-shell-preview.obj" /link /DEF:"$project/platforms/preview/windows/preview.def" /OUT:"$output/nabi-shell-preview.dll" ole32.lib shell32.lib user32.lib gdi32.lib msimg32.lib uuid.lib
if errorlevel 1 exit /b %errorlevel%
cl /nologo /std:c++17 /EHsc /W4 /MT /O2 /DUNICODE /D_UNICODE "$project/platforms/preview/windows/smoke.cpp" /Fo"$output/preview-smoke.obj" /Fe"$output/preview-smoke.exe" /link ole32.lib shlwapi.lib user32.lib gdi32.lib uuid.lib
exit /b %errorlevel%
"@ | Set-Content -LiteralPath $script -Encoding ascii
    & cmd /d /c $script
    if ($LASTEXITCODE) { throw 'Windows COM preview compilation failed.' }
    if ($Smoke) {
        if ($Architecture -eq 'arm64' -and $env:PROCESSOR_ARCHITECTURE -ne 'ARM64') { throw 'ARM64 smoke requires an ARM64 Windows host.' }
        & (Join-Path $output 'preview-smoke.exe') (Join-Path $output 'nabi-shell-preview.dll') (Join-Path $project 'assets/field-notes/01 Alpine Light.png')
        if ($LASTEXITCODE) { throw "COM preview smoke failed: $LASTEXITCODE" }
    }
} finally { Pop-Location }
