// PixLens PSD/PSB 资源管理器缩略图扩展（C++ COM 壳）
// 解码核心调用 Rust psd-capi（psd_capi.lib，MSVC ABI 无缝链接）。
//
// 注册（per-user）：
//   HKCU\Software\Classes\.psd\.psb\shellex\{E357FCCD-A995-4576-B01F-234630154E96} = {CLSID}
//   HKCU\Software\Classes\CLSID\{CLSID}\InprocServer32 = <本DLL>, ThreadingModel=Apartment

#include <windows.h>
#include <shobjidl.h>       // IShellItem, IInitializeWithStream/Item
#include <propsys.h>        // IInitializeWithFile
#include <thumbcache.h>     // IThumbnailProvider, WTSAT_UNKNOWN
#include <shlwapi.h>        // SHCreateMemStream
#include <gdiplus.h>
#include <vector>
#include <cstdio>
#pragma comment(lib, "gdiplus.lib")

// ── Rust 解码接口（独立 cdylib：pixlens_psd.dll，运行期动态加载） ──────
// 为什么动态加载：Rust staticlib 链入 C++ DLL 时其运行时初始化与 C++ CRT
// 在 COM 代理进程（DllHost）中冲突（实测栈溢出）；cdylib 独立初始化无此问题。
typedef int (*FnEmbeddedJpeg)(const unsigned char*, size_t, unsigned char*, size_t);
typedef int (*FnCompositeRgba)(const unsigned char*, size_t, unsigned, unsigned char*, size_t, unsigned*, unsigned*);

extern "C" IMAGE_DOS_HEADER __ImageBase;

static FnEmbeddedJpeg g_fn_embedded = nullptr;
static FnCompositeRgba g_fn_composite = nullptr;

static bool loadRustLib() {
    if (g_fn_composite) return true;
    // 与本 DLL 同目录的 pixlens_psd.dll
    wchar_t path[MAX_PATH];
    DWORD n = GetModuleFileNameW((HMODULE)&__ImageBase, path, MAX_PATH);
    if (n == 0 || n >= MAX_PATH) return false;
    wchar_t* slash = wcsrchr(path, L'\\');
    if (!slash) return false;
    wcscpy_s(slash + 1, MAX_PATH - (DWORD)(slash + 1 - path), L"pixlens_psd.dll");
    HMODULE h = LoadLibraryW(path);
    if (!h) return false;
    g_fn_embedded = (FnEmbeddedJpeg)GetProcAddress(h, "pixlens_psd_embedded_jpeg");
    g_fn_composite = (FnCompositeRgba)GetProcAddress(h, "pixlens_psd_composite_rgba");
    return g_fn_composite != nullptr;
}

// GDI+ 一次性初始化
struct GdiplusInit {
    ULONG_PTR token;
    GdiplusInit() {
        Gdiplus::GdiplusStartupInput si;
        Gdiplus::GdiplusStartup(&token, &si, nullptr);
    }
    ~GdiplusInit() { Gdiplus::GdiplusShutdown(token); }
};

// ── CLSID（与注册表一致，勿改） ─────────────
// {EE50AE86-B417-458F-BC3F-1F6C489E4D6C}
static const CLSID CLSID_PixLensThumb = {
    0x2B2E7C27, 0xBC52, 0x4521, {0x9A, 0x56, 0x87, 0xBC, 0x2D, 0xFC, 0x76, 0x39} };

#ifdef _DEBUG
static void dbg_log(const char* msg) {
    char path[MAX_PATH]; if (!GetEnvironmentVariableA("TEMP", path, MAX_PATH)) return;
    strcat_s(path, "\\pixlens_thumb_cpp.log");
    FILE* f = nullptr; if (fopen_s(&f, path, "a") == 0 && f) { fprintf(f, "[%lu] %s\n", GetCurrentProcessId(), msg); fclose(f); }
}
#else
#define dbg_log(msg)
#endif

// ── 缩略图提供者（多继承 COM） ─────────────
class ThumbnailProvider : public IThumbnailProvider,
                          public IInitializeWithStream,
                          public IInitializeWithFile,
                          public IInitializeWithItem {
    LONG m_rc;
    std::vector<unsigned char> m_data; // 初始化时缓存文件内容

public:
    ThumbnailProvider() : m_rc(1) {}

    // IUnknown
    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        *ppv = nullptr;
        if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_IThumbnailProvider)) {
            *ppv = static_cast<IThumbnailProvider*>(this);
        } else if (IsEqualIID(riid, IID_IInitializeWithStream)) {
            *ppv = static_cast<IInitializeWithStream*>(this);
        } else if (IsEqualIID(riid, IID_IInitializeWithFile)) {
            *ppv = static_cast<IInitializeWithFile*>(this);
        } else if (IsEqualIID(riid, IID_IInitializeWithItem)) {
            *ppv = static_cast<IInitializeWithItem*>(this);
        } else {
            return E_NOINTERFACE;
        }
        AddRef();
        return S_OK;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_rc); }
    STDMETHODIMP_(ULONG) Release() override {
        ULONG rc = InterlockedDecrement(&m_rc);
        if (rc == 0) delete this;
        return rc;
    }

    // IInitializeWithStream（Explorer 首选）
    // 注意：读流缓冲必须用堆——DllHost 的 COM 线程栈很小，
    // 1MB 栈数组会触发 __chkstk 栈溢出（此前所有版本崩溃的真正根因）
    STDMETHODIMP Initialize(IStream* stream, DWORD) override {
        if (!stream) return E_FAIL;
        m_data.clear();
        std::vector<unsigned char> buf(1 << 20);
        ULONG n = 0;
        for (;;) {
            HRESULT hr = stream->Read(buf.data(), (ULONG)buf.size(), &n);
            if (FAILED(hr)) return hr;
            if (n == 0) break;
            m_data.insert(m_data.end(), buf.begin(), buf.begin() + n);
            if (m_data.size() > (size_t)512 * 1024 * 1024) return E_FAIL;
        }
        return S_OK;
    }

    // IInitializeWithFile
    STDMETHODIMP Initialize(LPCWSTR path, DWORD) override {
        if (!path) return E_FAIL;
        HANDLE f = CreateFileW(path, GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                               nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
        if (f == INVALID_HANDLE_VALUE) return E_FAIL;
        m_data.clear();
        std::vector<unsigned char> buf(1 << 20);
        DWORD n = 0;
        while (ReadFile(f, buf.data(), (DWORD)buf.size(), &n, nullptr) && n > 0) {
            m_data.insert(m_data.end(), buf.begin(), buf.begin() + n);
            if (m_data.size() > (size_t)512 * 1024 * 1024) { CloseHandle(f); return E_FAIL; }
        }
        CloseHandle(f);
        return S_OK;
    }

    // IInitializeWithItem：从 IShellItem 取文件路径后走文件路径
    STDMETHODIMP Initialize(IShellItem* item, DWORD mode) override {
        if (!item) return E_FAIL;
        PWSTR pwsz = nullptr;
        HRESULT hr = item->GetDisplayName(SIGDN_FILESYSPATH, &pwsz);
        if (FAILED(hr) || !pwsz) return E_FAIL;
        hr = Initialize(pwsz, mode);
        CoTaskMemFree(pwsz);
        return hr;
    }

    // IThumbnailProvider
    STDMETHODIMP GetThumbnail(UINT cx, HBITMAP* phbmp, WTS_ALPHATYPE* pdwAlpha) override {
        if (!phbmp || !pdwAlpha || m_data.empty()) return E_FAIL;
        if (!loadRustLib()) return E_FAIL;

        // 1) 内嵌缩略图快路径（1036 JPEG → 系统 GDI+ 解码）
        int need = g_fn_embedded(m_data.data(), m_data.size(), nullptr, 0);
        if (need > 0) {
            std::vector<unsigned char> jpeg((size_t)need);
            int n = g_fn_embedded(m_data.data(), m_data.size(), jpeg.data(), jpeg.size());
            if (n > 0) {
                IStream* stm = SHCreateMemStream(jpeg.data(), (UINT)n);
                if (stm) {
                    Gdiplus::Bitmap bmp(stm, FALSE);
                    stm->Release();
                    if (bmp.GetLastStatus() == Gdiplus::Ok &&
                        bmp.GetHBITMAP(Gdiplus::Color(0, 0, 0), phbmp) == Gdiplus::Ok) {
                        *pdwAlpha = WTSAT_UNKNOWN;
                        return S_OK;
                    }
                }
            }
        }

        // 2) 合成图跨步解码（纯自研 Rust 路径，无第三方解码器）
        const size_t cap = (size_t)cx * cx * 4;
        std::vector<unsigned char> rgba(cap);
        unsigned w = 0, h = 0;
        if (g_fn_composite(m_data.data(), m_data.size(), cx, rgba.data(), cap, &w, &h) != 0) {
            return E_FAIL;
        }
        // RGBA → 32bpp top-down BGRA DIB
        BITMAPINFO bi = {};
        bi.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
        bi.bmiHeader.biWidth = (LONG)w;
        bi.bmiHeader.biHeight = -(LONG)h; // top-down
        bi.bmiHeader.biPlanes = 1;
        bi.bmiHeader.biBitCount = 32;
        bi.bmiHeader.biCompression = BI_RGB;
        void* bits = nullptr;
        HDC hdc = GetDC(nullptr);
        HBITMAP hbmp = CreateDIBSection(hdc, &bi, DIB_RGB_COLORS, &bits, nullptr, 0);
        ReleaseDC(nullptr, hdc);
        if (!hbmp || !bits) return E_FAIL;
        unsigned char* dst = (unsigned char*)bits;
        for (size_t i = 0; i < (size_t)w * h; i++) {
            dst[i * 4 + 0] = rgba[i * 4 + 2];
            dst[i * 4 + 1] = rgba[i * 4 + 1];
            dst[i * 4 + 2] = rgba[i * 4 + 0];
            dst[i * 4 + 3] = 0xFF;
        }
        *phbmp = hbmp;
        *pdwAlpha = WTSAT_UNKNOWN;
        return S_OK;
    }
};

// ── 类工厂 ─────────────────────────
class Factory : public IClassFactory {
    LONG m_rc;
public:
    Factory() : m_rc(1) {}

    STDMETHODIMP QueryInterface(REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        *ppv = nullptr;
        if (IsEqualIID(riid, IID_IUnknown) || IsEqualIID(riid, IID_IClassFactory)) {
            *ppv = static_cast<IClassFactory*>(this);
            AddRef();
            return S_OK;
        }
        return E_NOINTERFACE;
    }
    STDMETHODIMP_(ULONG) AddRef() override { return InterlockedIncrement(&m_rc); }
    STDMETHODIMP_(ULONG) Release() override {
        ULONG rc = InterlockedDecrement(&m_rc);
        if (rc == 0) delete this;
        return rc;
    }

    STDMETHODIMP CreateInstance(IUnknown* outer, REFIID riid, void** ppv) override {
        if (!ppv) return E_POINTER;
        *ppv = nullptr;
        if (outer) return CLASS_E_NOAGGREGATION;
        auto* p = new (std::nothrow) ThumbnailProvider();
        if (!p) return E_OUTOFMEMORY;
        HRESULT hr = p->QueryInterface(riid, ppv);
        p->Release();
        return hr;
    }
    STDMETHODIMP LockServer(BOOL) override { return S_OK; }
};

// ── DLL 导出 ─────────────────────────
STDAPI DllGetClassObject(REFCLSID rclsid, REFIID riid, void** ppv) {
    if (!ppv) return E_POINTER;
    *ppv = nullptr;
    if (!IsEqualCLSID(rclsid, CLSID_PixLensThumb)) return CLASS_E_CLASSNOTAVAILABLE;
    auto* f = new (std::nothrow) Factory();
    if (!f) return E_OUTOFMEMORY;
    HRESULT hr = f->QueryInterface(riid, ppv);
    f->Release();
    return hr;
}

STDAPI DllCanUnloadNow() { return S_FALSE; }
#pragma comment(linker, "/export:DllGetClassObject")
#pragma comment(linker, "/export:DllCanUnloadNow")
