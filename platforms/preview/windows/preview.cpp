#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <shobjidl.h>
#include <thumbcache.h>
#include <shlwapi.h>
#include <new>
#include <string>
#include <vector>
#include <algorithm>
#include <chrono>
#include "../nabi_preview.h"

// One stream-only COM class supplies both isolated shell interfaces.
static const CLSID CLSID_NabiPreview = {0x6b130ef4,0xe2c7,0x4bfd,{0xa5,0xa9,0x62,0xcd,0x3b,0xf8,0x2f,0x40}};
static HMODULE module;
static LONG objects = 0;
static LONG locks = 0;
static LONG decoderLoaded = 0;
static constexpr size_t MAX_INPUT = 64 * 1024 * 1024;
using DecodeBytes = int32_t (*)(const uint8_t *, size_t, const char *, uint32_t, NabiPreview *);
using FreePreview = void (*)(NabiPreview *);

struct Decoder {
    HMODULE library = nullptr;
    DecodeBytes decode = nullptr;
    FreePreview release = nullptr;
    Decoder() {
        wchar_t path[32768];
        DWORD count = GetModuleFileNameW(module, path, 32768);
        if (!count || count >= 32768) return;
        std::wstring name(path, count);
        size_t slash = name.find_last_of(L"\\/");
        if (slash == std::wstring::npos) return;
        name.resize(slash + 1);
        name += L"nabi_preview.dll";
        library = LoadLibraryExW(name.c_str(), nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
        if (!library) return;
        InterlockedExchange(&decoderLoaded, 1);
        decode = reinterpret_cast<DecodeBytes>(GetProcAddress(library, "nabi_preview_decode_bytes"));
        release = reinterpret_cast<FreePreview>(GetProcAddress(library, "nabi_preview_free"));
    }
};

static std::string extension(const std::vector<uint8_t>& bytes, IStream *stream) {
    if (bytes.size() >= 16 && memcmp(bytes.data() + 4, "ftyp", 4) == 0) {
        size_t brandEnd = std::min<size_t>(bytes.size(), 128);
        for (size_t offset = 8; offset + 4 <= brandEnd; offset += 4) {
            if (offset == 12) continue;
            if (memcmp(bytes.data() + offset, "avif", 4) == 0 || memcmp(bytes.data() + offset, "avis", 4) == 0) return "avif";
        }
    }
    if (bytes.size() >= 2 && bytes[0] == 0xff && bytes[1] == 0x0a) return "jxl";
    if (bytes.size() >= 12 && memcmp(bytes.data() + 4, "JXL ", 4) == 0) return "jxl";
    STATSTG stat = {};
    std::string result;
    if (SUCCEEDED(stream->Stat(&stat, STATFLAG_DEFAULT)) && stat.pwcsName) {
        const wchar_t *dot = wcsrchr(stat.pwcsName, L'.');
        if (dot && wcslen(dot) <= 17) {
            for (++dot; *dot; ++dot) {
                if ((*dot >= L'a' && *dot <= L'z') || (*dot >= L'A' && *dot <= L'Z') || (*dot >= L'0' && *dot <= L'9')) result.push_back(static_cast<char>(*dot));
                else { result.clear(); break; }
            }
        }
        CoTaskMemFree(stat.pwcsName);
    }
    if (!result.empty()) return result;
    std::string head(bytes.begin(), bytes.begin() + std::min<size_t>(bytes.size(), 512));
    if (head.find("<svg") != std::string::npos || head.find("<?xml") != std::string::npos) return "svg";
    return "bin"; // The shared image reader detects the remaining file signatures.
}

static HRESULT bitmap(IStream *stream, UINT side, HBITMAP *result, SIZE *dimensions) {
    if (!stream || !result || !dimensions || !side || side > 2048) return E_INVALIDARG;
    *result = nullptr;
    LARGE_INTEGER zero = {};
    HRESULT status = stream->Seek(zero, STREAM_SEEK_SET, nullptr);
    if (FAILED(status)) return status;
    std::vector<uint8_t> bytes;
    uint8_t block[64 * 1024];
    auto start = std::chrono::steady_clock::now();
    for (;;) {
        ULONG read = 0;
        status = stream->Read(block, sizeof(block), &read);
        if (FAILED(status)) return status;
        if (read > sizeof(block) || bytes.size() + read > MAX_INPUT) return HRESULT_FROM_WIN32(ERROR_FILE_TOO_LARGE);
        bytes.insert(bytes.end(), block, block + read);
        if (std::chrono::steady_clock::now() - start > std::chrono::seconds(10)) return HRESULT_FROM_WIN32(ERROR_TIMEOUT);
        if (!read || status == S_FALSE) break;
    }
    static Decoder decoder;
    if (!decoder.decode || !decoder.release) return HRESULT_FROM_WIN32(ERROR_MOD_NOT_FOUND);
    NabiPreview image = {};
    if (decoder.decode(bytes.data(), bytes.size(), extension(bytes, stream).c_str(), side, &image)) return E_FAIL;
    BITMAPINFO info = {};
    info.bmiHeader.biSize = sizeof(BITMAPINFOHEADER);
    info.bmiHeader.biWidth = static_cast<LONG>(image.width);
    info.bmiHeader.biHeight = -static_cast<LONG>(image.height);
    info.bmiHeader.biPlanes = 1;
    info.bmiHeader.biBitCount = 32;
    info.bmiHeader.biCompression = BI_RGB;
    void *pixels = nullptr;
    HBITMAP handle = CreateDIBSection(nullptr, &info, DIB_RGB_COLORS, &pixels, nullptr, 0);
    if (handle) {
        auto output = static_cast<uint8_t *>(pixels);
        for (size_t index = 0; index < image.length; index += 4) {
            unsigned alpha = image.rgba[index + 3];
            output[index] = static_cast<uint8_t>((image.rgba[index + 2] * alpha + 127) / 255);
            output[index + 1] = static_cast<uint8_t>((image.rgba[index + 1] * alpha + 127) / 255);
            output[index + 2] = static_cast<uint8_t>((image.rgba[index] * alpha + 127) / 255);
            output[index + 3] = static_cast<uint8_t>(alpha);
        }
        dimensions->cx = image.width; dimensions->cy = image.height;
        *result = handle;
    }
    decoder.release(&image);
    return handle ? S_OK : E_OUTOFMEMORY;
}

class Handler final : public IInitializeWithStream, public IThumbnailProvider,
    public IPreviewHandler, public IObjectWithSite, public IOleWindow, public IPreviewHandlerVisuals {
    LONG references = 1;
    IStream *stream = nullptr;
    IUnknown *site = nullptr;
    HWND parent = nullptr, window = nullptr;
    RECT bounds = {};
    HBITMAP image = nullptr;
    SIZE imageSize = {};
    COLORREF background = RGB(255,255,255);
    static LRESULT CALLBACK windowProc(HWND hwnd, UINT message, WPARAM wparam, LPARAM lparam) {
        Handler *self = reinterpret_cast<Handler *>(GetWindowLongPtrW(hwnd, GWLP_USERDATA));
        if (message == WM_NCCREATE) {
            self = static_cast<Handler *>(reinterpret_cast<CREATESTRUCTW *>(lparam)->lpCreateParams);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, reinterpret_cast<LONG_PTR>(self));
        }
        if (message == WM_PAINT && self) {
            PAINTSTRUCT paint;
            HDC dc = BeginPaint(hwnd, &paint);
            RECT area; GetClientRect(hwnd, &area);
            HBRUSH brush = CreateSolidBrush(self->background);
            FillRect(dc, &area, brush); DeleteObject(brush);
            if (self->image && self->imageSize.cx > 0 && self->imageSize.cy > 0) {
                double ratio = std::min(static_cast<double>(area.right) / self->imageSize.cx,
                                        static_cast<double>(area.bottom) / self->imageSize.cy);
                int width = static_cast<int>(self->imageSize.cx * ratio), height = static_cast<int>(self->imageSize.cy * ratio);
                HDC memory = CreateCompatibleDC(dc);
                HGDIOBJ previous = SelectObject(memory, self->image);
                BLENDFUNCTION blend = {AC_SRC_OVER, 0, 255, AC_SRC_ALPHA};
                AlphaBlend(dc, (area.right-width)/2, (area.bottom-height)/2, width, height,
                           memory, 0, 0, self->imageSize.cx, self->imageSize.cy, blend);
                SelectObject(memory, previous); DeleteDC(memory);
            }
            EndPaint(hwnd, &paint); return 0;
        }
        return DefWindowProcW(hwnd, message, wparam, lparam);
    }
public:
    Handler() { InterlockedIncrement(&objects); }
    ~Handler() { Unload(); if (site) site->Release(); InterlockedDecrement(&objects); }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void **value) override {
        if (!value) return E_POINTER; *value = nullptr;
        if (id == IID_IUnknown || id == IID_IInitializeWithStream) *value = static_cast<IInitializeWithStream *>(this);
        else if (id == __uuidof(IThumbnailProvider)) *value = static_cast<IThumbnailProvider *>(this);
        else if (id == IID_IPreviewHandler) *value = static_cast<IPreviewHandler *>(this);
        else if (id == IID_IObjectWithSite) *value = static_cast<IObjectWithSite *>(this);
        else if (id == IID_IOleWindow) *value = static_cast<IOleWindow *>(this);
        else if (id == IID_IPreviewHandlerVisuals) *value = static_cast<IPreviewHandlerVisuals *>(this);
        else return E_NOINTERFACE;
        AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return InterlockedIncrement(&references); }
    ULONG STDMETHODCALLTYPE Release() override { LONG count = InterlockedDecrement(&references); if (!count) delete this; return count; }
    HRESULT STDMETHODCALLTYPE Initialize(IStream *value, DWORD mode) override {
        if (stream) return HRESULT_FROM_WIN32(ERROR_ALREADY_INITIALIZED);
        if (!value || (mode & (STGM_WRITE | STGM_READWRITE))) return E_INVALIDARG;
        stream = value; stream->AddRef(); return S_OK;
    }
    HRESULT STDMETHODCALLTYPE GetThumbnail(UINT side, HBITMAP *value, WTS_ALPHATYPE *alpha) override {
        if (!value || !alpha) return E_POINTER;
        *value = nullptr; *alpha = WTSAT_UNKNOWN;
        try { SIZE size; HRESULT status = bitmap(stream, std::min(side, 2048u), value, &size);
            if (SUCCEEDED(status)) *alpha = WTSAT_ARGB; return status;
        } catch (...) { return E_OUTOFMEMORY; }
    }
    HRESULT STDMETHODCALLTYPE SetWindow(HWND value, const RECT *rect) override {
        if (!value || !rect) return E_INVALIDARG;
        parent = value; bounds = *rect;
        if (window) { ::SetParent(window, parent); return SetRect(rect); } return S_OK;
    }
    HRESULT STDMETHODCALLTYPE SetRect(const RECT *rect) override {
        if (!rect) return E_POINTER;
        bounds = *rect;
        if (window) MoveWindow(window, bounds.left, bounds.top, std::max(0L,bounds.right-bounds.left), std::max(0L,bounds.bottom-bounds.top), TRUE);
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE DoPreview() override {
        if (!parent || !stream) return E_UNEXPECTED;
        if (window) return S_OK;
        if (image) { DeleteObject(image); image = nullptr; }
        try { HRESULT status = bitmap(stream, 2048, &image, &imageSize); if (FAILED(status)) return status; }
        catch (...) { return E_OUTOFMEMORY; }
        WNDCLASSW type = {}; type.lpfnWndProc = windowProc; type.hInstance = module; type.lpszClassName = L"NabiViewPreview";
        type.hCursor = LoadCursorW(nullptr, IDC_ARROW);
        if (!RegisterClassW(&type) && GetLastError() != ERROR_CLASS_ALREADY_EXISTS) return HRESULT_FROM_WIN32(GetLastError());
        window = CreateWindowExW(0, type.lpszClassName, L"", WS_CHILD | WS_VISIBLE, bounds.left, bounds.top,
            std::max(0L,bounds.right-bounds.left), std::max(0L,bounds.bottom-bounds.top), parent, nullptr, module, this);
        return window ? S_OK : HRESULT_FROM_WIN32(GetLastError());
    }
    HRESULT STDMETHODCALLTYPE Unload() override {
        if (window) { DestroyWindow(window); window = nullptr; }
        if (image) { DeleteObject(image); image = nullptr; }
        if (stream) { stream->Release(); stream = nullptr; }
        return S_OK;
    }
    HRESULT STDMETHODCALLTYPE SetFocus() override { if (window) ::SetFocus(window); return S_OK; }
    HRESULT STDMETHODCALLTYPE QueryFocus(HWND *value) override { if (!value) return E_POINTER; *value = ::GetFocus(); return S_OK; }
    HRESULT STDMETHODCALLTYPE TranslateAccelerator(MSG *message) override {
        if (!message) return E_POINTER;
        IPreviewHandlerFrame *frame = nullptr;
        if (!site || FAILED(site->QueryInterface(IID_PPV_ARGS(&frame)))) return S_FALSE;
        HRESULT status = frame->TranslateAccelerator(message); frame->Release(); return status;
    }
    HRESULT STDMETHODCALLTYPE SetSite(IUnknown *value) override { if (value) value->AddRef(); if (site) site->Release(); site=value; return S_OK; }
    HRESULT STDMETHODCALLTYPE GetSite(REFIID id, void **value) override { if (!value) return E_POINTER; *value=nullptr; return site ? site->QueryInterface(id,value) : E_FAIL; }
    HRESULT STDMETHODCALLTYPE GetWindow(HWND *value) override { if (!value) return E_POINTER; *value=window; return S_OK; }
    HRESULT STDMETHODCALLTYPE ContextSensitiveHelp(BOOL) override { return E_NOTIMPL; }
    HRESULT STDMETHODCALLTYPE SetBackgroundColor(COLORREF value) override { background=value; if (window) InvalidateRect(window,nullptr,TRUE); return S_OK; }
    HRESULT STDMETHODCALLTYPE SetFont(const LOGFONTW *) override { return S_OK; }
    HRESULT STDMETHODCALLTYPE SetTextColor(COLORREF) override { return S_OK; }
};

class Factory final : public IClassFactory {
    LONG references=1;
public:
    Factory() { InterlockedIncrement(&objects); }
    ~Factory() { InterlockedDecrement(&objects); }
    HRESULT STDMETHODCALLTYPE QueryInterface(REFIID id, void **value) override {
        if (!value) return E_POINTER; *value=nullptr;
        if (id != IID_IUnknown && id != IID_IClassFactory) return E_NOINTERFACE;
        *value=static_cast<IClassFactory *>(this); AddRef(); return S_OK;
    }
    ULONG STDMETHODCALLTYPE AddRef() override { return InterlockedIncrement(&references); }
    ULONG STDMETHODCALLTYPE Release() override { LONG count=InterlockedDecrement(&references); if (!count) delete this; return count; }
    HRESULT STDMETHODCALLTYPE CreateInstance(IUnknown *outer, REFIID id, void **value) override {
        if (!value) return E_POINTER; *value=nullptr;
        if (outer) return CLASS_E_NOAGGREGATION;
        auto object=new(std::nothrow) Handler(); if (!object) return E_OUTOFMEMORY;
        HRESULT status=object->QueryInterface(id,value); object->Release(); return status;
    }
    HRESULT STDMETHODCALLTYPE LockServer(BOOL lock) override { if (lock) InterlockedIncrement(&locks); else InterlockedDecrement(&locks); return S_OK; }
};
extern "C" HRESULT WINAPI DllGetClassObject(REFCLSID clsid, REFIID id, void **value) {
    if (!value) return E_POINTER;
    *value = nullptr;
    if (clsid != CLSID_NabiPreview) return CLASS_E_CLASSNOTAVAILABLE;
    auto factory=new(std::nothrow) Factory(); if (!factory) return E_OUTOFMEMORY;
    HRESULT status=factory->QueryInterface(id,value); factory->Release(); return status;
}
extern "C" HRESULT WINAPI DllCanUnloadNow() {
    // Rust/native codec TLS may outlive a call; keep both modules loaded until
    // the isolated system host exits, while Unload frees all per-file pixels.
    if (objects != 0 || locks != 0 || decoderLoaded != 0) return S_FALSE;
    UnregisterClassW(L"NabiViewPreview", module);
    return S_OK;
}
BOOL WINAPI DllMain(HINSTANCE instance, DWORD reason, LPVOID) {
    if (reason == DLL_PROCESS_ATTACH) { module=instance; DisableThreadLibraryCalls(instance); }
    return TRUE;
}
