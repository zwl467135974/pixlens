// PixLens PSD/PSB 资源管理器缩略图扩展（C++ COM 壳）
// 解码核心经 LoadLibrary 动态加载 Rust cdylib（pixlens_psd.dll）——
// staticlib 直链的 CRT 运行时初始化在 DllHost 中冲突，cdylib 分离则无此问题。
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
// 为什么走文件路径而非 IStream 流：mmap 零拷贝 + 按需分页，GB 级 PSB 不吃内存，
// 且无流式读取的 512MB 上限。
typedef int (*FnEmbeddedJpegFile)(const char* path, unsigned char* out, size_t cap);
typedef int (*FnEmbeddedJpegMem)(const unsigned char* data, size_t len, unsigned char* out, size_t cap);
typedef int (*FnCompositeRgbaFile)(const char* path, unsigned cx,
                                   unsigned char* out, size_t cap, unsigned* w, unsigned* h);
typedef int (*FnCompositeRgbaMem)(const unsigned char* data, size_t len, unsigned cx,
                                   unsigned char* out, size_t cap, unsigned* w, unsigned* h);

extern "C" IMAGE_DOS_HEADER __ImageBase;

static FnEmbeddedJpegFile g_fn_embedded = nullptr;
static FnEmbeddedJpegMem g_fn_embedded_mem = nullptr;
static FnCompositeRgbaFile g_fn_composite = nullptr;
static FnCompositeRgbaMem g_fn_composite_mem = nullptr;

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
    g_fn_embedded = (FnEmbeddedJpegFile)GetProcAddress(h, "pixlens_psd_embedded_jpeg_from_file");
    g_fn_composite = (FnCompositeRgbaFile)GetProcAddress(h, "pixlens_psd_composite_rgba_from_file");
    g_fn_embedded_mem = (FnEmbeddedJpegMem)GetProcAddress(h, "pixlens_psd_embedded_jpeg");
    g_fn_composite_mem = (FnCompositeRgbaMem)GetProcAddress(h, "pixlens_psd_composite_rgba");
    return g_fn_composite != nullptr && g_fn_embedded_mem != nullptr;
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

// ── CLSID（与注册表/nsis-hooks 一致，勿改） ─────────────
// {2B2E7C27-BC52-4521-9A56-87BC2DFC7639}
static const CLSID CLSID_PixLensThumb = {
    0x2B2E7C27, 0xBC52, 0x4521, {0x9A, 0x56, 0x87, 0xBC, 0x2D, 0xFC, 0x76, 0x39} };

// ── 缩略图提供者（多继承 COM） ─────────────
// 初始化双模式：
//   Shell 绑定要求 IInitializeWithStream 存在（不提供则整个 handler 不绑定），
//   stream 路径读数据（截断上限）；File/Item 路径存文件路径，解码时 mmap
//   按需分页——任意大小的 PSD/PSB 都不在本对象中驻留完整文件内容。
class ThumbnailProvider : public IThumbnailProvider,
                          public IInitializeWithStream,
                          public IInitializeWithFile,
                          public IInitializeWithItem {
    LONG m_rc;
    std::vector<char> m_path; // UTF-8 文件路径（File/Item 初始化）
    std::vector<unsigned char> m_data; // stream 数据（≤512MB 截断）

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

    // IInitializeWithStream（Shell 绑定必需；读流缓冲必须堆分配——DllHost 的
    // COM 线程栈小，大栈数组触发 __chkstk 栈溢出，此前所有版本崩溃的根因）。
    // 截断上限 512MB：避免 DllHost 为单个缩略图提交 GB 级内存；超限文件头部
    // 的 1036 内嵌缩略图仍可用（资源段位于文件前部）。
    STDMETHODIMP Initialize(IStream* stream, DWORD) override {
        if (!stream) return E_FAIL;
        try {
            m_data.clear();
            const size_t CAP = (size_t)512 * 1024 * 1024;
            std::vector<unsigned char> buf(1 << 20);
            ULONG n = 0;
            for (;;) {
                if (m_data.size() >= CAP) break;
                size_t want = buf.size() < CAP - m_data.size() ? buf.size() : CAP - m_data.size();
                HRESULT hr = stream->Read(buf.data(), (ULONG)want, &n);
                if (FAILED(hr)) return hr;
                if (n == 0) break;
                m_data.insert(m_data.end(), buf.begin(), buf.begin() + n);
            }
            return S_OK;
        } catch (...) { return E_FAIL; } // COM 边界不得泄漏 C++ 异常
    }

    // IInitializeWithFile
    STDMETHODIMP Initialize(LPCWSTR path, DWORD) override {
        if (!path) return E_FAIL;
        return setPath(path);
    }

    // IInitializeWithItem：从 IShellItem 取文件路径后同上
    STDMETHODIMP Initialize(IShellItem* item, DWORD mode) override {
        if (!item) return E_FAIL;
        PWSTR pwsz = nullptr;
        HRESULT hr = item->GetDisplayName(SIGDN_FILESYSPATH, &pwsz);
        if (FAILED(hr) || !pwsz) return E_FAIL;
        hr = setPath(pwsz);
        CoTaskMemFree(pwsz);
        return hr;
    }

    // 宽字符路径 → UTF-8 存储
    HRESULT setPath(LPCWSTR wpath) {
        int need = WideCharToMultiByte(CP_UTF8, 0, wpath, -1, nullptr, 0, nullptr, nullptr);
        if (need <= 0) return E_FAIL;
        m_path.resize((size_t)need);
        WideCharToMultiByte(CP_UTF8, 0, wpath, -1, m_path.data(), need, nullptr, nullptr);
        return S_OK;
    }

    // IThumbnailProvider
    STDMETHODIMP GetThumbnail(UINT cx, HBITMAP* phbmp, WTS_ALPHATYPE* pdwAlpha) override {
        if (!phbmp || !pdwAlpha) return E_FAIL;
        try {
            if (!loadRustLib()) return E_FAIL;

            // 头部数据视图（指针引用，不拷贝）：m_path 时读文件头 2MB，否则用
            // stream 数据——1036 内嵌缩略图位于文件头部资源段，2MB 覆盖绝大多数文件
            std::vector<unsigned char> headFile;
            const unsigned char* head = nullptr;
            size_t headLen = 0;
            if (!m_path.empty()) {
                HANDLE f = CreateFileA(m_path.data(), GENERIC_READ, FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
                                       nullptr, OPEN_EXISTING, FILE_ATTRIBUTE_NORMAL, nullptr);
                if (f != INVALID_HANDLE_VALUE) {
                    const size_t HEAD = 2 << 20;
                    DWORD n = 0;
                    headFile.resize(HEAD);
                    if (!ReadFile(f, headFile.data(), (DWORD)headFile.size(), &n, nullptr) || n == 0) n = 0;
                    headFile.resize(n);
                    CloseHandle(f);
                    head = headFile.data();
                    headLen = headFile.size();
                }
            } else if (!m_data.empty()) {
                head = m_data.data();
                headLen = m_data.size();
            }
            if (!head || headLen == 0) return E_FAIL;

            // 1) 内嵌缩略图快路径（1036 JPEG → 系统 GDI+ 解码）
            int need = g_fn_embedded_mem(head, headLen, nullptr, 0);
            if (need > 0) {
                std::vector<unsigned char> jpeg((size_t)need);
                int n = g_fn_embedded_mem(head, headLen, jpeg.data(), jpeg.size());
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

            // 2) 合成图跨步解码：路径模式 mmap 任意大小；stream 模式仅当数据完整
            const size_t cap = (size_t)cx * cx * 4;
            std::vector<unsigned char> rgba(cap);
            unsigned w = 0, h = 0;
            int cres = -1;
            if (!m_path.empty()) {
                cres = g_fn_composite(m_path.data(), cx, rgba.data(), cap, &w, &h);
            } else {
                cres = g_fn_composite_mem(head, headLen, cx, rgba.data(), cap, &w, &h);
            }
            if (cres != 0) return E_FAIL;

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
        } catch (...) { return E_FAIL; } // COM 边界不得泄漏 C++ 异常
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
