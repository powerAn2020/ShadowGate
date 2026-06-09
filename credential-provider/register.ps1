param(
    [string]$DllPath = "$PSScriptRoot\ShadowGateCredentialProvider.dll"
)

$ErrorActionPreference = "Stop"
$clsid = "{8F59E94D-1B6B-4B89-95E1-35F3E1C8A7B1}"
$providerName = "ShadowGate Credential Provider"

if (-not ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Administrator privileges are required to register a Credential Provider."
}

if (-not (Test-Path -LiteralPath $DllPath)) {
    throw "DLL not found: $DllPath"
}

& regsvr32.exe /s $DllPath

$base = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\$clsid"
New-Item -Path $base -Force | Out-Null
New-ItemProperty -Path $base -Name "(default)" -Value $providerName -PropertyType String -Force | Out-Null

Write-Host "Registered $providerName ($clsid)"

