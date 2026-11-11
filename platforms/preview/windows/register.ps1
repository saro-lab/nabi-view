[CmdletBinding()]
param([switch]$Uninstall)
$ErrorActionPreference='Stop'
# Explicit per-user registration for the portable package. MSIX uses its manifest.
$clsid='{6B130EF4-E2C7-4BFD-A5A9-62CD3BF82F40}'
$base='HKCU:\Software\Classes'
$class="$base\CLSID\$clsid"
$dll=Join-Path $PSScriptRoot 'nabi-shell-preview.dll'
$extensions=@('avif','jxl','qoi','exr','hdr','dds','ff')
$interfaces=@('{E357FCCD-A995-4576-B01F-234630154E96}','{8895B1C6-B41F-4C1C-A562-0D564250836F}')
$handlers='HKCU:\Software\Microsoft\Windows\CurrentVersion\PreviewHandlers'
function DefaultValue($path) {
    if (Test-Path -LiteralPath $path) { return (Get-Item -LiteralPath $path).GetValue('') }
    return $null
}
if ($Uninstall) {
    if ((DefaultValue "$class\InprocServer32") -ne $dll) { throw 'This installation does not own the registered preview class.' }
    foreach ($extension in $extensions) {
        foreach ($interface in $interfaces) {
            $path="$base\.$extension\shellex\$interface"
            if ((DefaultValue $path) -eq $clsid) { Remove-Item -LiteralPath $path }
        }
    }
    Remove-Item -LiteralPath $class -Recurse
    if (Test-Path $handlers) { Remove-ItemProperty -LiteralPath $handlers -Name $clsid -ErrorAction SilentlyContinue }
} else {
    if (-not (Test-Path -LiteralPath $dll) -or -not (Test-Path (Join-Path $PSScriptRoot 'nabi_preview.dll'))) { throw 'Both preview DLLs must be next to this script.' }
    $previous=DefaultValue "$class\InprocServer32"
    if ($previous -and $previous -ne $dll) { throw 'Uninstall the previous preview registration before moving its files.' }
    New-Item -Path "$class\InprocServer32" -Force | Out-Null
    Set-Item -LiteralPath $class -Value 'NABI VIEW preview'
    Set-Item -LiteralPath "$class\InprocServer32" -Value $dll
    New-ItemProperty -LiteralPath "$class\InprocServer32" -Name ThreadingModel -Value Apartment -PropertyType String -Force | Out-Null
    # Windows' low-integrity preview surrogate. Do not opt out of isolation.
    New-ItemProperty -LiteralPath $class -Name AppID -Value '{6D2B5079-2F0B-48DD-AB7F-97CEC514D30B}' -PropertyType String -Force | Out-Null
    New-Item -Path $handlers -Force | Out-Null
    New-ItemProperty -LiteralPath $handlers -Name $clsid -Value 'NABI VIEW preview' -PropertyType String -Force | Out-Null
    foreach ($extension in $extensions) {
        foreach ($interface in $interfaces) {
            $path="$base\.$extension\shellex\$interface"
            $existing=DefaultValue $path
            # Respect existing providers, including machine-wide registration.
            $effective=DefaultValue "Registry::HKEY_CLASSES_ROOT\.$extension\shellex\$interface"
            if (($existing -and $existing -ne $clsid) -or ($effective -and $effective -ne $clsid)) {
                Write-Output "Kept existing .$extension handler $interface"
                continue
            }
            New-Item -Path $path -Force | Out-Null
            Set-Item -LiteralPath $path -Value $clsid
        }
    }
}
Add-Type -TypeDefinition 'using System; using System.Runtime.InteropServices; public static class NabiShellNotify { [DllImport("shell32.dll")] public static extern void SHChangeNotify(uint eventId, uint flags, IntPtr first, IntPtr second); }'
[NabiShellNotify]::SHChangeNotify(0x08000000,0,[IntPtr]::Zero,[IntPtr]::Zero)
