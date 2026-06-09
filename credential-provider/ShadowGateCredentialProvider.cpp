#include <Windows.h>
#include <credentialprovider.h>
#include <wincred.h>
#include <ntsecapi.h>
#include <ShlObj.h>
#include <strsafe.h>

#include <cctype>
#include <cstring>
#include <fstream>
#include <new>
#include <string>

// CLSID: {8F59E94D-1B6B-4B89-95E1-35F3E1C8A7B1}
const CLSID CLSID_ShadowGateCredentialProvider =
    {0x8f59e94d, 0x1b6b, 0x4b89, {0x95, 0xe1, 0x35, 0xf3, 0xe1, 0xc8, 0xa7, 0xb1}};

static const wchar_t kProviderName[] = L"ShadowGate Credential Provider";
static const wchar_t kCredentialTarget[] = L"ShadowGate:WindowsUnlock";
static long g_refCount = 0;
static HMODULE g_module = nullptr;

enum FieldId {
    FIELD_TITLE = 0,
    FIELD_STATUS = 1,
    FIELD_HINT = 2,
    FIELD_COUNT = 3,
};

struct FieldSpec {
    CREDENTIAL_PROVIDER_FIELD_TYPE type;
    const wchar_t* label;
};

static const FieldSpec kFields[FIELD_COUNT] = {
    {CPFT_LARGE_TEXT, L"ShadowGate"},
    {CPFT_SMALL_TEXT, L"Waiting for BLE authorization"},
    {CPFT_SMALL_TEXT, L"Unlock is available after Android proximity verification."},
};

template <typename T>
static void SafeRelease(T** object) {
    if (object && *object) {
        (*object)->Release();
        *object = nullptr;
    }
}

static PWSTR DuplicateString(const wchar_t* value) {
    if (!value) {
        value = L"";
    }
    size_t chars = wcslen(value) + 1;
    auto out = static_cast<PWSTR>(CoTaskMemAlloc(chars * sizeof(wchar_t)));
    if (out) {
        StringCchCopyW(out, chars, value);
    }
    return out;
}

static HRESULT GetModulePath(wchar_t* path, DWORD count) {
    if (!GetModuleFileNameW(g_module, path, count)) {
        return HRESULT_FROM_WIN32(GetLastError());
    }
    return S_OK;
}

static unsigned long long UnixMillisNow() {
    FILETIME ft = {};
    GetSystemTimeAsFileTime(&ft);
    ULARGE_INTEGER ticks = {};
    ticks.LowPart = ft.dwLowDateTime;
    ticks.HighPart = ft.dwHighDateTime;
    constexpr unsigned long long kUnixEpochFiletime = 116444736000000000ULL;
    return (ticks.QuadPart - kUnixEpochFiletime) / 10000ULL;
}

static std::wstring ProgramDataPath(const wchar_t* leaf) {
    PWSTR programData = nullptr;
    std::wstring path;
    if (SUCCEEDED(SHGetKnownFolderPath(FOLDERID_ProgramData, 0, nullptr, &programData))) {
        path = programData;
        CoTaskMemFree(programData);
        path += L"\\ShadowGate\\";
        path += leaf;
    }
    return path;
}

static bool ExtractJsonU64(const std::string& json, const char* key, unsigned long long* value) {
    std::string needle = std::string("\"") + key + "\"";
    size_t pos = json.find(needle);
    if (pos == std::string::npos) {
        return false;
    }
    pos = json.find(':', pos + needle.size());
    if (pos == std::string::npos) {
        return false;
    }
    ++pos;
    while (pos < json.size() && isspace(static_cast<unsigned char>(json[pos]))) {
        ++pos;
    }
    unsigned long long result = 0;
    bool sawDigit = false;
    while (pos < json.size() && isdigit(static_cast<unsigned char>(json[pos]))) {
        sawDigit = true;
        result = (result * 10ULL) + static_cast<unsigned long long>(json[pos] - '0');
        ++pos;
    }
    if (!sawDigit) {
        return false;
    }
    *value = result;
    return true;
}

static bool IsBleAuthorized() {
    std::wstring authPath = ProgramDataPath(L"credential_auth.json");
    if (authPath.empty()) {
        return false;
    }

    std::ifstream file(authPath);
    if (!file) {
        return false;
    }

    std::string content((std::istreambuf_iterator<char>(file)), std::istreambuf_iterator<char>());
    unsigned long long authorizedUntil = 0;
    if (!ExtractJsonU64(content, "authorized_until_ms", &authorizedUntil)) {
        return false;
    }
    return authorizedUntil > UnixMillisNow();
}

static bool ReadStoredCredential(std::wstring* username, std::wstring* password) {
    PCREDENTIALW credential = nullptr;
    if (!CredReadW(kCredentialTarget, CRED_TYPE_GENERIC, 0, &credential)) {
        return false;
    }

    if (credential->UserName) {
        *username = credential->UserName;
    }
    if (credential->CredentialBlob && credential->CredentialBlobSize >= sizeof(wchar_t)) {
        size_t chars = credential->CredentialBlobSize / sizeof(wchar_t);
        password->assign(reinterpret_cast<const wchar_t*>(credential->CredentialBlob), chars);
        while (!password->empty() && password->back() == L'\0') {
            password->pop_back();
        }
    }

    CredFree(credential);
    return !username->empty() && !password->empty();
}

static HRESULT RetrieveNegotiateAuthPackage(ULONG* authPackage) {
    HANDLE lsa = nullptr;
    NTSTATUS status = LsaConnectUntrusted(&lsa);
    if (status != 0) {
        return HRESULT_FROM_WIN32(LsaNtStatusToWinError(status));
    }

    LSA_STRING packageName = {};
    packageName.Buffer = const_cast<PCHAR>("Negotiate");
    packageName.Length = static_cast<USHORT>(strlen(packageName.Buffer));
    packageName.MaximumLength = packageName.Length + 1;

    status = LsaLookupAuthenticationPackage(lsa, &packageName, authPackage);
    LsaDeregisterLogonProcess(lsa);
    return status == 0 ? S_OK : HRESULT_FROM_WIN32(LsaNtStatusToWinError(status));
}

class ShadowGateCredential final : public ICredentialProviderCredential {
public:
    explicit ShadowGateCredential(CREDENTIAL_PROVIDER_USAGE_SCENARIO scenario)
        : _refCount(1), _scenario(scenario), _events(nullptr) {
        InterlockedIncrement(&g_refCount);
    }

    ~ShadowGateCredential() {
        SafeRelease(&_events);
        InterlockedDecrement(&g_refCount);
    }

    IFACEMETHODIMP QueryInterface(REFIID iid, void** object) override {
        if (!object) {
            return E_POINTER;
        }
        *object = nullptr;
        if (IsEqualIID(iid, IID_IUnknown) || IsEqualIID(iid, __uuidof(ICredentialProviderCredential))) {
            *object = static_cast<ICredentialProviderCredential*>(this);
            AddRef();
            return S_OK;
        }
        return E_NOINTERFACE;
    }

    IFACEMETHODIMP_(ULONG) AddRef() override {
        return InterlockedIncrement(&_refCount);
    }

    IFACEMETHODIMP_(ULONG) Release() override {
        long count = InterlockedDecrement(&_refCount);
        if (!count) {
            delete this;
        }
        return count;
    }

    IFACEMETHODIMP Advise(ICredentialProviderCredentialEvents* events) override {
        SafeRelease(&_events);
        _events = events;
        if (_events) {
            _events->AddRef();
        }
        return S_OK;
    }

    IFACEMETHODIMP UnAdvise() override {
        SafeRelease(&_events);
        return S_OK;
    }

    IFACEMETHODIMP SetSelected(BOOL* autoLogon) override {
        if (autoLogon) {
            *autoLogon = FALSE;
        }
        return S_OK;
    }

    IFACEMETHODIMP SetDeselected() override {
        return S_OK;
    }

    IFACEMETHODIMP GetFieldState(
        DWORD fieldId,
        CREDENTIAL_PROVIDER_FIELD_STATE* state,
        CREDENTIAL_PROVIDER_FIELD_INTERACTIVE_STATE* interactiveState) override {
        if (!state || !interactiveState || fieldId >= FIELD_COUNT) {
            return E_INVALIDARG;
        }
        *state = CPFS_DISPLAY_IN_SELECTED_TILE;
        *interactiveState = CPFIS_NONE;
        return S_OK;
    }

    IFACEMETHODIMP GetStringValue(DWORD fieldId, PWSTR* value) override {
        if (!value || fieldId >= FIELD_COUNT) {
            return E_INVALIDARG;
        }

        const wchar_t* text = kFields[fieldId].label;
        std::wstring dynamicText;
        if (fieldId == FIELD_STATUS) {
            if (_scenario != CPUS_UNLOCK_WORKSTATION) {
                dynamicText = L"Sign in normally first; ShadowGate unlock is for locked sessions.";
            } else if (!IsBleAuthorized()) {
                dynamicText = L"Waiting for a fresh Android BLE authorization.";
            } else {
                std::wstring username;
                std::wstring password;
                dynamicText = ReadStoredCredential(&username, &password)
                    ? L"BLE authorization accepted. Select this tile to unlock."
                    : L"Windows credential is missing. Open ShadowGate settings first.";
            }
            text = dynamicText.c_str();
        }

        *value = DuplicateString(text);
        return *value ? S_OK : E_OUTOFMEMORY;
    }

    IFACEMETHODIMP GetBitmapValue(DWORD, HBITMAP*) override {
        return E_NOTIMPL;
    }

    IFACEMETHODIMP GetCheckboxValue(DWORD, BOOL*, PWSTR*) override {
        return E_NOTIMPL;
    }

    IFACEMETHODIMP GetSubmitButtonValue(DWORD, DWORD*) override {
        return E_NOTIMPL;
    }

    IFACEMETHODIMP GetComboBoxValueCount(DWORD, DWORD*, DWORD*) override {
        return E_NOTIMPL;
    }

    IFACEMETHODIMP GetComboBoxValueAt(DWORD, DWORD, PWSTR*) override {
        return E_NOTIMPL;
    }

    IFACEMETHODIMP SetStringValue(DWORD, PCWSTR) override {
        return E_NOTIMPL;
    }

    IFACEMETHODIMP SetCheckboxValue(DWORD, BOOL) override {
        return E_NOTIMPL;
    }

    IFACEMETHODIMP SetComboBoxSelectedValue(DWORD, DWORD) override {
        return E_NOTIMPL;
    }

    IFACEMETHODIMP CommandLinkClicked(DWORD) override {
        return E_NOTIMPL;
    }

    IFACEMETHODIMP GetSerialization(
        CREDENTIAL_PROVIDER_GET_SERIALIZATION_RESPONSE* response,
        CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION* serialization,
        PWSTR* statusText,
        CREDENTIAL_PROVIDER_STATUS_ICON* statusIcon) override {
        if (!response || !serialization || !statusText || !statusIcon) {
            return E_POINTER;
        }
        ZeroMemory(serialization, sizeof(*serialization));
        *response = CPGSR_NO_CREDENTIAL_NOT_FINISHED;
        *statusIcon = CPSI_WARNING;
        *statusText = nullptr;

        if (_scenario != CPUS_UNLOCK_WORKSTATION) {
            *statusText = DuplicateString(L"ShadowGate unlock is available after the first normal sign-in.");
            return S_OK;
        }
        if (!IsBleAuthorized()) {
            *statusText = DuplicateString(L"No fresh BLE authorization is available.");
            return S_OK;
        }

        std::wstring username;
        std::wstring password;
        if (!ReadStoredCredential(&username, &password)) {
            *statusText = DuplicateString(L"ShadowGate Windows credential is not configured.");
            return S_OK;
        }

        ULONG authPackage = 0;
        HRESULT hr = RetrieveNegotiateAuthPackage(&authPackage);
        if (FAILED(hr)) {
            *statusText = DuplicateString(L"Unable to resolve Windows authentication package.");
            return S_OK;
        }

        DWORD packedSize = 0;
        CredPackAuthenticationBufferW(0, username.data(), password.data(), nullptr, &packedSize);
        if (GetLastError() != ERROR_INSUFFICIENT_BUFFER) {
            *statusText = DuplicateString(L"Unable to prepare ShadowGate unlock credentials.");
            return S_OK;
        }

        auto packed = static_cast<BYTE*>(CoTaskMemAlloc(packedSize));
        if (!packed) {
            return E_OUTOFMEMORY;
        }

        if (!CredPackAuthenticationBufferW(0, username.data(), password.data(), packed, &packedSize)) {
            CoTaskMemFree(packed);
            *statusText = DuplicateString(L"Unable to pack ShadowGate unlock credentials.");
            return S_OK;
        }

        serialization->ulAuthenticationPackage = authPackage;
        serialization->clsidCredentialProvider = CLSID_ShadowGateCredentialProvider;
        serialization->cbSerialization = packedSize;
        serialization->rgbSerialization = packed;
        *statusIcon = CPSI_SUCCESS;
        *response = CPGSR_RETURN_CREDENTIAL_FINISHED;
        return S_OK;
    }

    IFACEMETHODIMP ReportResult(
        NTSTATUS,
        NTSTATUS,
        PWSTR* statusText,
        CREDENTIAL_PROVIDER_STATUS_ICON* statusIcon) override {
        if (statusText) {
            *statusText = nullptr;
        }
        if (statusIcon) {
            *statusIcon = CPSI_NONE;
        }
        return S_OK;
    }

private:
    long _refCount;
    CREDENTIAL_PROVIDER_USAGE_SCENARIO _scenario;
    ICredentialProviderCredentialEvents* _events;
};

class ShadowGateProvider final : public ICredentialProvider {
public:
    ShadowGateProvider()
        : _refCount(1), _scenario(CPUS_INVALID), _credential(nullptr) {
        InterlockedIncrement(&g_refCount);
    }

    ~ShadowGateProvider() {
        SafeRelease(&_credential);
        InterlockedDecrement(&g_refCount);
    }

    IFACEMETHODIMP QueryInterface(REFIID iid, void** object) override {
        if (!object) {
            return E_POINTER;
        }
        *object = nullptr;
        if (IsEqualIID(iid, IID_IUnknown) || IsEqualIID(iid, __uuidof(ICredentialProvider))) {
            *object = static_cast<ICredentialProvider*>(this);
            AddRef();
            return S_OK;
        }
        return E_NOINTERFACE;
    }

    IFACEMETHODIMP_(ULONG) AddRef() override {
        return InterlockedIncrement(&_refCount);
    }

    IFACEMETHODIMP_(ULONG) Release() override {
        long count = InterlockedDecrement(&_refCount);
        if (!count) {
            delete this;
        }
        return count;
    }

    IFACEMETHODIMP SetUsageScenario(CREDENTIAL_PROVIDER_USAGE_SCENARIO scenario, DWORD) override {
        SafeRelease(&_credential);
        _scenario = scenario;
        if (scenario != CPUS_UNLOCK_WORKSTATION && scenario != CPUS_LOGON) {
            return E_NOTIMPL;
        }
        _credential = new (std::nothrow) ShadowGateCredential(scenario);
        return _credential ? S_OK : E_OUTOFMEMORY;
    }

    IFACEMETHODIMP SetSerialization(const CREDENTIAL_PROVIDER_CREDENTIAL_SERIALIZATION*) override {
        return S_OK;
    }

    IFACEMETHODIMP Advise(ICredentialProviderEvents*, UINT_PTR) override {
        return S_OK;
    }

    IFACEMETHODIMP UnAdvise() override {
        return S_OK;
    }

    IFACEMETHODIMP GetFieldDescriptorCount(DWORD* count) override {
        if (!count) {
            return E_POINTER;
        }
        *count = FIELD_COUNT;
        return S_OK;
    }

    IFACEMETHODIMP GetFieldDescriptorAt(DWORD fieldId, CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR** descriptor) override {
        if (!descriptor || fieldId >= FIELD_COUNT) {
            return E_INVALIDARG;
        }
        auto out = static_cast<CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR*>(
            CoTaskMemAlloc(sizeof(CREDENTIAL_PROVIDER_FIELD_DESCRIPTOR)));
        if (!out) {
            return E_OUTOFMEMORY;
        }
        out->dwFieldID = fieldId;
        out->cpft = kFields[fieldId].type;
        out->guidFieldType = GUID_NULL;
        out->pszLabel = DuplicateString(kFields[fieldId].label);
        if (!out->pszLabel) {
            CoTaskMemFree(out);
            return E_OUTOFMEMORY;
        }
        *descriptor = out;
        return S_OK;
    }

    IFACEMETHODIMP GetCredentialCount(DWORD* count, DWORD* defaultCredential, BOOL* autoLogonWithDefault) override {
        if (!count || !defaultCredential || !autoLogonWithDefault) {
            return E_POINTER;
        }
        *count = _credential ? 1 : 0;
        *defaultCredential = 0;
        *autoLogonWithDefault = FALSE;
        return S_OK;
    }

    IFACEMETHODIMP GetCredentialAt(DWORD index, ICredentialProviderCredential** credential) override {
        if (!credential) {
            return E_POINTER;
        }
        *credential = nullptr;
        if (index != 0 || !_credential) {
            return E_INVALIDARG;
        }
        _credential->AddRef();
        *credential = _credential;
        return S_OK;
    }

private:
    long _refCount;
    CREDENTIAL_PROVIDER_USAGE_SCENARIO _scenario;
    ShadowGateCredential* _credential;
};

class ShadowGateClassFactory final : public IClassFactory {
public:
    ShadowGateClassFactory() : _refCount(1) {
        InterlockedIncrement(&g_refCount);
    }

    ~ShadowGateClassFactory() {
        InterlockedDecrement(&g_refCount);
    }

    IFACEMETHODIMP QueryInterface(REFIID iid, void** object) override {
        if (!object) {
            return E_POINTER;
        }
        *object = nullptr;
        if (IsEqualIID(iid, IID_IUnknown) || IsEqualIID(iid, IID_IClassFactory)) {
            *object = static_cast<IClassFactory*>(this);
            AddRef();
            return S_OK;
        }
        return E_NOINTERFACE;
    }

    IFACEMETHODIMP_(ULONG) AddRef() override {
        return InterlockedIncrement(&_refCount);
    }

    IFACEMETHODIMP_(ULONG) Release() override {
        long count = InterlockedDecrement(&_refCount);
        if (!count) {
            delete this;
        }
        return count;
    }

    IFACEMETHODIMP CreateInstance(IUnknown* outer, REFIID iid, void** object) override {
        if (outer) {
            return CLASS_E_NOAGGREGATION;
        }
        auto provider = new (std::nothrow) ShadowGateProvider();
        if (!provider) {
            return E_OUTOFMEMORY;
        }
        HRESULT hr = provider->QueryInterface(iid, object);
        provider->Release();
        return hr;
    }

    IFACEMETHODIMP LockServer(BOOL lock) override {
        if (lock) {
            InterlockedIncrement(&g_refCount);
        } else {
            InterlockedDecrement(&g_refCount);
        }
        return S_OK;
    }

private:
    long _refCount;
};

static HRESULT RegisterComServer() {
    wchar_t modulePath[MAX_PATH] = {};
    HRESULT hr = GetModulePath(modulePath, ARRAYSIZE(modulePath));
    if (FAILED(hr)) {
        return hr;
    }

    wchar_t clsidString[64] = {};
    if (!StringFromGUID2(CLSID_ShadowGateCredentialProvider, clsidString, ARRAYSIZE(clsidString))) {
        return E_FAIL;
    }

    wchar_t keyPath[256] = {};
    hr = StringCchPrintfW(keyPath, ARRAYSIZE(keyPath), L"CLSID\\%s", clsidString);
    if (FAILED(hr)) {
        return hr;
    }

    HKEY key = nullptr;
    LSTATUS status = RegCreateKeyExW(HKEY_CLASSES_ROOT, keyPath, 0, nullptr, 0, KEY_WRITE, nullptr, &key, nullptr);
    if (status != ERROR_SUCCESS) {
        return HRESULT_FROM_WIN32(status);
    }
    RegSetValueExW(key, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(kProviderName),
        static_cast<DWORD>((wcslen(kProviderName) + 1) * sizeof(wchar_t)));

    HKEY inproc = nullptr;
    status = RegCreateKeyExW(key, L"InprocServer32", 0, nullptr, 0, KEY_WRITE, nullptr, &inproc, nullptr);
    if (status == ERROR_SUCCESS) {
        RegSetValueExW(inproc, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(modulePath),
            static_cast<DWORD>((wcslen(modulePath) + 1) * sizeof(wchar_t)));
        RegSetValueExW(inproc, L"ThreadingModel", 0, REG_SZ, reinterpret_cast<const BYTE*>(L"Apartment"),
            static_cast<DWORD>(sizeof(L"Apartment")));
        RegCloseKey(inproc);
    }
    RegCloseKey(key);
    return HRESULT_FROM_WIN32(status);
}

static HRESULT RegisterCredentialProvider() {
    wchar_t clsidString[64] = {};
    if (!StringFromGUID2(CLSID_ShadowGateCredentialProvider, clsidString, ARRAYSIZE(clsidString))) {
        return E_FAIL;
    }

    wchar_t keyPath[300] = {};
    HRESULT hr = StringCchPrintfW(
        keyPath,
        ARRAYSIZE(keyPath),
        L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Authentication\\Credential Providers\\%s",
        clsidString);
    if (FAILED(hr)) {
        return hr;
    }

    HKEY key = nullptr;
    LSTATUS status = RegCreateKeyExW(HKEY_LOCAL_MACHINE, keyPath, 0, nullptr, 0, KEY_WRITE, nullptr, &key, nullptr);
    if (status != ERROR_SUCCESS) {
        return HRESULT_FROM_WIN32(status);
    }
    RegSetValueExW(key, nullptr, 0, REG_SZ, reinterpret_cast<const BYTE*>(kProviderName),
        static_cast<DWORD>((wcslen(kProviderName) + 1) * sizeof(wchar_t)));
    RegCloseKey(key);
    return S_OK;
}

BOOL APIENTRY DllMain(HMODULE module, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) {
        g_module = module;
        DisableThreadLibraryCalls(module);
    }
    return TRUE;
}

STDAPI DllCanUnloadNow() {
    return g_refCount == 0 ? S_OK : S_FALSE;
}

STDAPI DllGetClassObject(REFCLSID clsid, REFIID iid, void** object) {
    if (!object) {
        return E_POINTER;
    }
    *object = nullptr;
    if (!IsEqualCLSID(clsid, CLSID_ShadowGateCredentialProvider)) {
        return CLASS_E_CLASSNOTAVAILABLE;
    }
    auto factory = new (std::nothrow) ShadowGateClassFactory();
    if (!factory) {
        return E_OUTOFMEMORY;
    }
    HRESULT hr = factory->QueryInterface(iid, object);
    factory->Release();
    return hr;
}

STDAPI DllRegisterServer() {
    HRESULT hr = RegisterComServer();
    if (FAILED(hr)) {
        return hr;
    }
    return RegisterCredentialProvider();
}

STDAPI DllUnregisterServer() {
    wchar_t clsidString[64] = {};
    if (!StringFromGUID2(CLSID_ShadowGateCredentialProvider, clsidString, ARRAYSIZE(clsidString))) {
        return E_FAIL;
    }

    wchar_t comPath[256] = {};
    if (SUCCEEDED(StringCchPrintfW(comPath, ARRAYSIZE(comPath), L"CLSID\\%s", clsidString))) {
        RegDeleteTreeW(HKEY_CLASSES_ROOT, comPath);
    }

    wchar_t providerPath[300] = {};
    if (SUCCEEDED(StringCchPrintfW(
            providerPath,
            ARRAYSIZE(providerPath),
            L"SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Authentication\\Credential Providers\\%s",
            clsidString))) {
        RegDeleteTreeW(HKEY_LOCAL_MACHINE, providerPath);
    }
    return S_OK;
}
