# ShadowGate Credential Provider

This directory contains the Windows Credential Provider integration for ShadowGate.

The provider is intentionally fail-closed:

- It only considers unlock when `%ProgramData%\ShadowGate\credential_auth.json` exists.
- The file must contain `authorized_until_ms`, `device_hash`, and `auth_nonce`.
- The current time must be before `authorized_until_ms`.
- User credentials are expected to be stored by the Tauri administrator setup flow in Windows Credential Manager.

The first implementation target is unlock for an already signed-in and locked Windows session. Cold boot sign-in should show the tile but require ShadowGate setup first.

The C++ COM implementation follows the Microsoft V2 Credential Provider sample shape:

- `DllGetClassObject`
- `DllCanUnloadNow`
- `IClassFactory`
- `ICredentialProvider`
- `ICredentialProviderCredential`
- COM registration under `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers`

When selected on the lock screen, the tile checks for fresh BLE authorization, reads the saved `ShadowGate:WindowsUnlock` generic credential, resolves the Negotiate authentication package, and packs the username/password with `CredPackAuthenticationBufferW`. It does not collect credentials on the secure desktop.

Build:

```powershell
& "C:\Program Files (x86)\Microsoft Visual Studio\2022\BuildTools\MSBuild\Current\Bin\amd64\MSBuild.exe" .\ShadowGateCredentialProvider.vcxproj /p:Configuration=Release /p:Platform=x64
```

Register/unregister from an elevated PowerShell session:

```powershell
.\register.ps1 -DllPath .\x64\Release\ShadowGateCredentialProvider.dll
.\unregister.ps1 -DllPath .\x64\Release\ShadowGateCredentialProvider.dll
```
