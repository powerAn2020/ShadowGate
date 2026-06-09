#include <Windows.h>
#include <ShlObj.h>
#include <strsafe.h>

// ShadowGate Credential Provider scaffold.
// CLSID: {8F59E94D-1B6B-4B89-95E1-35F3E1C8A7B1}

static long g_refCount = 0;

#ifndef RETURN_IF_FAILED
#define RETURN_IF_FAILED(hr_)      \
    do {                           \
        HRESULT _hr = (hr_);       \
        if (FAILED(_hr)) return _hr; \
    } while (0)
#endif

const CLSID CLSID_ShadowGateCredentialProvider =
    {0x8f59e94d, 0x1b6b, 0x4b89, {0x95, 0xe1, 0x35, 0xf3, 0xe1, 0xc8, 0xa7, 0xb1}};

BOOL APIENTRY DllMain(HMODULE module, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) {
        DisableThreadLibraryCalls(module);
    }
    return TRUE;
}

STDAPI DllCanUnloadNow() {
    return g_refCount == 0 ? S_OK : S_FALSE;
}

STDAPI DllGetClassObject(REFCLSID clsid, REFIID, void** object) {
    if (!object) {
        return E_POINTER;
    }
    *object = nullptr;
    if (clsid != CLSID_ShadowGateCredentialProvider) {
        return CLASS_E_CLASSNOTAVAILABLE;
    }

    // Full IClassFactory/ICredentialProvider implementation is intentionally
    // left for the Windows SDK sample integration step. This export makes the
    // DLL registration scaffold explicit while failing closed at runtime.
    return CLASS_E_CLASSNOTAVAILABLE;
}

static HRESULT GetModulePath(wchar_t* path, DWORD count) {
    HMODULE module = nullptr;
    if (!GetModuleHandleExW(
            GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
            reinterpret_cast<LPCWSTR>(&GetModulePath),
            &module)) {
        return HRESULT_FROM_WIN32(GetLastError());
    }
    return GetModuleFileNameW(module, path, count) ? S_OK : HRESULT_FROM_WIN32(GetLastError());
}

STDAPI DllRegisterServer() {
    wchar_t modulePath[MAX_PATH] = {};
    RETURN_IF_FAILED(GetModulePath(modulePath, ARRAYSIZE(modulePath)));

    wchar_t clsidString[64] = {};
    RETURN_IF_FAILED(StringFromGUID2(CLSID_ShadowGateCredentialProvider, clsidString, ARRAYSIZE(clsidString)) ? S_OK : E_FAIL);

    wchar_t keyPath[256] = {};
    RETURN_IF_FAILED(StringCchPrintfW(keyPath, ARRAYSIZE(keyPath), L"CLSID\\%s", clsidString));

    HKEY key = nullptr;
    LSTATUS status = RegCreateKeyExW(HKEY_CLASSES_ROOT, keyPath, 0, nullptr, 0, KEY_WRITE, nullptr, &key, nullptr);
    if (status != ERROR_SUCCESS) {
        return HRESULT_FROM_WIN32(status);
    }
    RegSetValueExW(key, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(L"ShadowGate Credential Provider"), sizeof(L"ShadowGate Credential Provider"));

    HKEY inproc = nullptr;
    status = RegCreateKeyExW(key, L"InprocServer32", 0, nullptr, 0, KEY_WRITE, nullptr, &inproc, nullptr);
    if (status == ERROR_SUCCESS) {
        RegSetValueExW(inproc, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(modulePath), static_cast<DWORD>((wcslen(modulePath) + 1) * sizeof(wchar_t)));
        RegSetValueExW(inproc, L"ThreadingModel", 0, REG_SZ, reinterpret_cast<const BYTE*>(L"Apartment"), sizeof(L"Apartment"));
        RegCloseKey(inproc);
    }
    RegCloseKey(key);
    return HRESULT_FROM_WIN32(status);
}

STDAPI DllUnregisterServer() {
    wchar_t clsidString[64] = {};
    RETURN_IF_FAILED(StringFromGUID2(CLSID_ShadowGateCredentialProvider, clsidString, ARRAYSIZE(clsidString)) ? S_OK : E_FAIL);

    wchar_t keyPath[256] = {};
    RETURN_IF_FAILED(StringCchPrintfW(keyPath, ARRAYSIZE(keyPath), L"CLSID\\%s", clsidString));
    RegDeleteTreeW(HKEY_CLASSES_ROOT, keyPath);
    return S_OK;
}
