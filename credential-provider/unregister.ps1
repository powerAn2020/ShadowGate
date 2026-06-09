param(
    [string]$DllPath = "$PSScriptRoot\ShadowGateCredentialProvider.dll"
)

$ErrorActionPreference = "Stop"
$clsid = "{8F59E94D-1B6B-4B89-95E1-35F3E1C8A7B1}"

if (-not ([Security.Principal.WindowsPrincipal] [Security.Principal.WindowsIdentity]::GetCurrent()).IsInRole([Security.Principal.WindowsBuiltInRole]::Administrator)) {
    throw "Administrator privileges are required to unregister a Credential Provider."
}

if (Test-Path -LiteralPath $DllPath) {
    & regsvr32.exe /s /u $DllPath
}

$base = "HKLM:\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers\$clsid"
if (Test-Path $base) {
    Remove-Item -Path $base -Recurse -Force
}

Write-Host "Unregistered ShadowGate Credential Provider ($clsid)"

