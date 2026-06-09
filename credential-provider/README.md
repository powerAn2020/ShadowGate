# ShadowGate Credential Provider

This directory contains the Windows Credential Provider integration scaffold for ShadowGate.

The provider is intentionally fail-closed:

- It only considers unlock when `%ProgramData%\ShadowGate\credential_auth.json` exists.
- The file must contain `authorized_until_ms`, `device_hash`, and `auth_nonce`.
- The current time must be before `authorized_until_ms`.
- User credentials are expected to be stored by the Tauri administrator setup flow in Windows Credential Manager.

The first implementation target is unlock for an already signed-in and locked Windows session. Cold boot sign-in should show the tile but require ShadowGate setup first.

The C++ COM implementation is based on the Microsoft V2 Credential Provider sample shape:

- `DllGetClassObject`
- `DllCanUnloadNow`
- COM registration under `HKLM\SOFTWARE\Microsoft\Windows\CurrentVersion\Authentication\Credential Providers`

The current scaffold builds the DLL entry points and registration scripts. The full LogonUI field model and credential packing should be completed against the Windows SDK sample on a Windows development machine.

