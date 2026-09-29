// PixLens PSD/PSB/视频 资源管理器缩略图扩展（C++ COM 壳）
// 解码核心经 LoadLibrary 动态加载 Rust cdylib（pixlens_psd.dll）——
// staticlib 直链的 CRT 运行时初始化在 DllHost 中冲突，cdylib 分离则无此问题。
// 视频（mp4 等）代表帧由本文件直调 Media Foundation 提取（系统解码器）。
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
#include <mfapi.h>
#include <mfidl.h>
#include <mfreadwrite.h>
#include <propvarutil.h>
#include <vector>
#include <cstdio>
#include <cstring>
#pragma comment(lib, "gdiplus.lib")
#pragma comment(lib, "mfplat.lib")
#pragma comment(lib, "mfreadwrite.lib")
#pragma comment(lib, "mfuuid.lib")

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
// 流式合成：head 前缀 + C 回调（IStream seek/read）按需读采样行
typedef int (*FnCompositeRgbaStream)(const unsigned char* head, size_t head_len, unsigned long long total,
                                     void* ctx, int (*seek)(void*, unsigned long long),
                                     int (*read)(void*, unsigned char*, size_t),
                                     unsigned cx, unsigned char* out, size_t cap, unsigned* w, unsigned* h);

extern "C" IMAGE_DOS_HEADER __ImageBase;

static FnEmbeddedJpegFile g_fn_embedded = nullptr;
static FnEmbeddedJpegMem g_fn_embedded_mem = nullptr;
static FnCompositeRgbaFile g_fn_composite = nullptr;
static FnCompositeRgbaMem g_fn_composite_mem = nullptr;
static FnCompositeRgbaStream g_fn_composite_stream = nullptr;

static bool loadRustLib() {
    if (g_fn_composite_stream) return true;
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
    g_fn_composite_stream = (FnCompositeRgbaStream)GetProcAddress(h, "pixlens_psd_composite_rgba_stream");
    return g_fn_composite != nullptr && g_fn_embedded_mem != nullptr && g_fn_composite_stream != nullptr;
}

// GDI+ 惰性一次性初始化（C++11 magic static 线程安全；勿在 DllMain 阶段做）
static void ensure_gdiplus() {
    struct GdiplusInit {
        ULONG_PTR token;
        GdiplusInit() {
            Gdiplus::GdiplusStartupInput si;
            Gdiplus::GdiplusStartup(&token, &si, nullptr);
        }
        ~GdiplusInit() { Gdiplus::GdiplusShutdown(token); }
    };
    static GdiplusInit init;
}

// ── 视频（MP4 等）代表帧：Media Foundation（系统解码器） ─────────────
// Explorer 自带的 shell32 视频缩略图依赖机器上的解码环境（纯净系统对
// H.264/新编码经常出不来），这里由我们统一提供：与 PixLens 应用内
// （Rust 侧 codecs/video.rs）同一套取帧策略——ENABLE_VIDEO_PROCESSING
// 转 RGB32（旋转元数据自动应用）+ seek 1s 取代表帧（首帧常是黑屏）。

// MFStartup 只进不出（无析构：DLL 卸载顺序不保证 mfplat 仍在，进程退出回收）
static bool ensure_mf() {
    static bool ok = [] { return SUCCEEDED(MFStartup(MF_VERSION, MFSTARTUP_LITE)); }();
    return ok;
}

static bool isVideoExt(const wchar_t* ext) {
    if (!ext || !*ext) return false;
    wchar_t e[8] = {};
    for (int i = 0; ext[i] && i < 7; i++) e[i] = (wchar_t)towlower(ext[i]);
    static const wchar_t* const kExts[] = { L"mp4", L"m4v", L"mov", L"webm", L"mkv", L"avi", L"wmv" };
    for (auto x : kExts) if (!wcscmp(e, x)) return true;
    return false;
}

static bool isVideoExtUtf8(const char* path) {
    if (!path) return false;
    const char* dot = strrchr(path, '.');
    if (!dot) return false;
    wchar_t w[8] = {};
    MultiByteToWideChar(CP_UTF8, 0, dot + 1, -1, w, 8);
    return isVideoExt(w);
}

// MF RGB32 输出（BGRA、stride 可负=bottom-up）→ 32bpp top-down HBITMAP
static HBITMAP bgraToHbitmap(const BYTE* src, size_t len, UINT32 w, UINT32 h, LONG stride) {
    if (!w || !h) return nullptr;
    if (stride >= 0 && (size_t)stride * h > len) return nullptr;
    if (stride < 0 && (size_t)(-stride) * h > len) return nullptr;
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
    if (!hbmp || !bits) { if (hbmp) DeleteObject(hbmp); return nullptr; }
    const size_t rowLen = (size_t)w * 4;
    for (UINT32 y = 0; y < h; y++) {
        size_t srcOff = stride >= 0 ? (size_t)y * stride
                                    : (size_t)(h - 1 - y) * (size_t)(-stride);
        if (srcOff + rowLen > len) break;
        memcpy((BYTE*)bits + (size_t)y * rowLen, src + srcOff, rowLen);
    }
    return hbmp;
}

// 超过请求尺寸时 GDI+ 高质量缩小（IThumbnailProvider 契约：返回 ≤ cx）
static HBITMAP shrinkToCx(HBITMAP hbmp, UINT cx) {
    BITMAP bm = {};
    if (!GetObjectW(hbmp, sizeof(bm), &bm) || bm.bmWidth <= 0 || bm.bmHeight <= 0) return hbmp;
    LONG w = bm.bmWidth, h = bm.bmHeight;
    if ((UINT)w <= cx && (UINT)h <= cx) return hbmp;
    UINT tw = w, th = h;
    if (w >= h && (UINT)w > cx) { tw = cx; th = (UINT)((unsigned long long)h * cx / w); }
    else if ((UINT)h > cx)      { th = cx; tw = (UINT)((unsigned long long)w * cx / h); }
    if (tw < 1) tw = 1;
    if (th < 1) th = 1;
    Gdiplus::Bitmap src(hbmp, nullptr);
    Gdiplus::Bitmap dst((INT)tw, (INT)th, PixelFormat32bppRGB);
    Gdiplus::Graphics g(&dst);
    g.SetInterpolationMode(Gdiplus::InterpolationModeHighQualityBilinear);
    if (g.DrawImage(&src, 0, 0, (INT)tw, (INT)th) != Gdiplus::Ok) return hbmp;
    HBITMAP out = nullptr;
    if (dst.GetHBITMAP(Gdiplus::Color(0, 0, 0), &out) != Gdiplus::Ok || !out) return hbmp;
    DeleteObject(hbmp);
    return out;
}

// 读源阅读器一帧 → HBITMAP（含 RGB32 转换与 seek）
static HRESULT mfFrameToBitmap(IMFSourceReader* reader, UINT cx, HBITMAP* phbmp) {
    static const DWORD kVideo = (DWORD)MF_SOURCE_READER_FIRST_VIDEO_STREAM;
    HRESULT hr = reader->SetStreamSelection(kVideo, TRUE);
    if (FAILED(hr)) return hr;
    reader->SetStreamSelection((DWORD)MF_SOURCE_READER_FIRST_AUDIO_STREAM, FALSE); // 无音频流时忽略

    // 原生分辨率护栏（防御异常流：8K 以下才处理）
    IMFMediaType* native = nullptr;
    hr = reader->GetCurrentMediaType(kVideo, &native);
    if (FAILED(hr)) return hr;
    UINT64 sz = 0;
    native->GetUINT64(MF_MT_FRAME_SIZE, &sz);
    native->Release();
    UINT32 w = (UINT32)(sz >> 32), h = (UINT32)sz;
    if (!w || !h || (unsigned long long)w * h > 40000000ull) return E_FAIL;

    // 目标 RGB32：源阅读器自动插入 video processor（必须显式开启，否则
    // SetCurrentMediaType 直接 0xC00D36B4）；老式转换会自动应用旋转元数据
    IMFMediaType* target = nullptr;
    hr = MFCreateMediaType(&target);
    if (FAILED(hr)) return hr;
    target->SetGUID(MF_MT_MAJOR_TYPE, MFMediaType_Video);
    target->SetGUID(MF_MT_SUBTYPE, MFVideoFormat_RGB32);
    hr = reader->SetCurrentMediaType(kVideo, nullptr, target);
    target->Release();
    if (FAILED(hr)) return hr;

    // seek 1s 取代表帧；不支持 seek 的容器失败即从首帧读
    PROPVARIANT pos;
    PropVariantInit(&pos);
    pos.vt = VT_I8;
    pos.hVal.QuadPart = 10000000; // 100ns 单位
    reader->SetCurrentPosition(GUID_NULL, pos);

    for (int i = 0; i < 240; i++) {
        DWORD flags = 0;
        IMFSample* sample = nullptr;
        hr = reader->ReadSample(kVideo, 0, nullptr, &flags, nullptr, &sample);
        if (FAILED(hr)) return hr;
        if (flags & MF_SOURCE_READERF_ENDOFSTREAM) {
            if (sample) sample->Release();
            return E_FAIL;
        }
        if (!sample) continue; // seek 缝隙的 STREAMTICK
        IMFMediaBuffer* buf = nullptr;
        hr = sample->ConvertToContiguousBuffer(&buf);
        sample->Release();
        if (FAILED(hr)) return hr;
        BYTE* ptr = nullptr;
        DWORD maxLen = 0, curLen = 0;
        hr = buf->Lock(&ptr, &maxLen, &curLen);
        if (SUCCEEDED(hr)) {
            // 转换后宽高（旋转已应用则对调）与 stride
            UINT32 ow = w, oh = h;
            LONG stride = (LONG)w * 4;
            IMFMediaType* cur = nullptr;
            if (SUCCEEDED(reader->GetCurrentMediaType(kVideo, &cur))) {
                UINT64 os = 0;
                UINT32 st = 0;
                if (SUCCEEDED(cur->GetUINT64(MF_MT_FRAME_SIZE, &os))) {
                    ow = (UINT32)(os >> 32);
                    oh = (UINT32)os;
                }
                if (SUCCEEDED(cur->GetUINT32(MF_MT_DEFAULT_STRIDE, &st))) stride = (LONG)st;
                cur->Release();
            }
            HBITMAP hbmp = bgraToHbitmap(ptr, curLen, ow, oh, stride);
            buf->Unlock();
            if (hbmp) {
                *phbmp = shrinkToCx(hbmp, cx);
                return S_OK;
            }
        }
        buf->Release();
        if (FAILED(hr)) return hr;
    }
    return E_FAIL;
}

// 视频缩略图入口：path 模式按 URL 打开；stream 模式包 MFByteStream（Shell 绑定
// 走 IInitializeWithStream，视频文件与 PSD 同一条初始化路径）
static HRESULT videoThumbnail(UINT cx, const char* path8, IStream* stream, HBITMAP* phbmp) {
    if (!ensure_mf()) return E_FAIL;
    ensure_gdiplus(); // shrinkToCx 需要（GDI+ 未初始化时静默失败返回原图）
    IMFAttributes* attrs = nullptr;
    HRESULT hr = MFCreateAttributes(&attrs, 1);
    if (FAILED(hr)) return hr;
    attrs->SetUINT32(MF_SOURCE_READER_ENABLE_VIDEO_PROCESSING, TRUE);

    IMFSourceReader* reader = nullptr;
    if (path8 && *path8) {
        wchar_t wpath[1024];
        int n = MultiByteToWideChar(CP_UTF8, 0, path8, -1, wpath, 1024);
        if (n <= 0) { attrs->Release(); return E_FAIL; }
        hr = MFCreateSourceReaderFromURL(wpath, attrs, &reader);
    } else if (stream) {
        // Initialize 阶段读过流（或位置未知）：回到 0 再交给 MF
        LARGE_INTEGER zero = {};
        stream->Seek(zero, STREAM_SEEK_SET, nullptr);
        IMFByteStream* bs = nullptr;
        hr = MFCreateMFByteStreamOnStreamEx(stream, &bs);
        if (SUCCEEDED(hr)) {
            hr = MFCreateSourceReaderFromByteStream(bs, attrs, &reader);
            bs->Release();
        }
    } else {
        hr = E_FAIL;
    }
    attrs->Release();
    if (FAILED(hr) || !reader) return FAILED(hr) ? hr : E_FAIL;

    hr = mfFrameToBitmap(reader, cx, phbmp);
    reader->Release();
    return hr;
}

// IStream → Rust 字节源回调（绝对定位 / 读满；IStream 非线程安全，仅串行调用）
static int istream_seek_cb(void* ctx, unsigned long long pos) {
    LARGE_INTEGER li;
    memcpy(&li, &pos, sizeof(li));
    return SUCCEEDED(((IStream*)ctx)->Seek(li, STREAM_SEEK_SET, nullptr)) ? 0 : -1;
}
static int istream_read_cb(void* ctx, unsigned char* buf, size_t len) {
    ULONG got = 0;
    if (len > 0xFFFFFFFFull) return -1;
    return SUCCEEDED(((IStream*)ctx)->Read(buf, (ULONG)len, &got)) && got == len ? 0 : -1;
}

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
    std::vector<unsigned char> m_data; // 文件头部（stream 初始化截断 2MB / path 初始化读 2MB）
    IStream* m_stream = nullptr; // stream 模式持有的流引用（合成图 seek 采样用）
    unsigned long long m_total = 0; // 完整文件长度（STATSTG）
    bool m_isVideo = false; // stream 名字识别为视频（Initialize 设置；path 模式按扩展判断）

public:
    ThumbnailProvider() : m_rc(1) {}
    ~ThumbnailProvider() {
        if (m_stream) m_stream->Release();
    }

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
    // 视频流：只记扩展名与长度，不读头（代表帧由 MF 直取，避免大文件白读）。
    // PSD/PSB：读 8MB 头部（1036 内嵌缩略图 + 段定位字段都在前部），流引用
    // 保留给合成图 seek 采样——任意大小文件都有界内存，不再整流读入。
    STDMETHODIMP Initialize(IStream* stream, DWORD) override {
        if (!stream) return E_FAIL;
        try {
            STATSTG stg = {};
            if (FAILED(stream->Stat(&stg, STATFLAG_DEFAULT))) return E_FAIL;
            m_total = stg.cbSize.QuadPart;
            bool video = false;
            if (stg.pwcsName) {
                const wchar_t* name = wcsrchr(stg.pwcsName, L'\\');
                name = name ? name + 1 : stg.pwcsName;
                const wchar_t* dot = wcsrchr(name, L'.');
                video = isVideoExt(dot ? dot + 1 : nullptr);
                CoTaskMemFree(stg.pwcsName);
            }
            stream->AddRef();
            if (m_stream) m_stream->Release();
            m_stream = stream;
            m_data.clear();
            if (video) {
                m_isVideo = true;
                return S_OK;
            }
            m_isVideo = false;
            // 8MB 头：真实文件的资源段可达数 MB（大 ICC 等），1036 需在头内命中；
            // 布局字段与合成数据另经流 seek 按需读取，不受此限制
            const size_t HEAD = (size_t)8 << 20;
            std::vector<unsigned char> buf(1 << 20);
            ULONG n = 0;
            for (;;) {
                if (m_data.size() >= HEAD || m_data.size() >= m_total) break;
                size_t want = buf.size() < HEAD - m_data.size() ? buf.size() : HEAD - m_data.size();
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
            // 视频分流：MF 代表帧（系统解码器），失败回落系统图标
            if (m_isVideo || isVideoExtUtf8(m_path.empty() ? nullptr : m_path.data())) {
                HRESULT hr = videoThumbnail(
                    cx, m_path.empty() ? nullptr : m_path.data(), m_stream, phbmp);
                if (SUCCEEDED(hr)) {
                    *pdwAlpha = WTSAT_UNKNOWN;
                    return S_OK;
                }
                return E_FAIL;
            }

            if (!loadRustLib()) return E_FAIL;
            ensure_gdiplus();

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

            // 2) 合成图跨步解码：路径模式 mmap 任意大小；stream 模式经 seek 回调
            //    只读采样行（2GB PSB 也只触碰几 MB，替代整流读入）
            const size_t cap = (size_t)cx * cx * 4;
            std::vector<unsigned char> rgba(cap);
            unsigned w = 0, h = 0;
            int cres = -1;
            if (!m_path.empty()) {
                cres = g_fn_composite(m_path.data(), cx, rgba.data(), cap, &w, &h);
            } else if (m_stream && g_fn_composite_stream) {
                cres = g_fn_composite_stream(m_data.data(), m_data.size(), m_total,
                                             m_stream, istream_seek_cb, istream_read_cb,
                                             cx, rgba.data(), cap, &w, &h);
            } else if (!m_data.empty() && m_data.size() >= m_total) {
                cres = g_fn_composite_mem(m_data.data(), m_data.size(), cx, rgba.data(), cap, &w, &h);
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
