#define WIN32_LEAN_AND_MEAN
#define NOMINMAX
#include <windows.h>
#include <shobjidl.h>
#include <thumbcache.h>
#include <shlwapi.h>
#include <cstdio>

// Exercise the real COM DLL without writing any registration or user settings.
int wmain(int argc, wchar_t **argv) {
    if (argc != 3) return 2;
    if (FAILED(CoInitializeEx(nullptr, COINIT_APARTMENTTHREADED))) return 3;
    auto library = LoadLibraryExW(argv[1], nullptr, LOAD_LIBRARY_SEARCH_DLL_LOAD_DIR | LOAD_LIBRARY_SEARCH_DEFAULT_DIRS);
    if (!library) return 4;
    using GetClass = HRESULT (WINAPI *)(REFCLSID, REFIID, void **);
    auto getClass = reinterpret_cast<GetClass>(GetProcAddress(library, "DllGetClassObject"));
    if (!getClass) return 5;
    const CLSID clsid = {0x6b130ef4,0xe2c7,0x4bfd,{0xa5,0xa9,0x62,0xcd,0x3b,0xf8,0x2f,0x40}};
    IClassFactory *factory = nullptr;
    IInitializeWithStream *initialize = nullptr;
    IThumbnailProvider *thumbnail = nullptr;
    IPreviewHandler *preview = nullptr;
    IStream *stream = nullptr;
    if (FAILED(getClass(clsid, IID_PPV_ARGS(&factory)))) return 6;
    if (FAILED(factory->CreateInstance(nullptr, IID_PPV_ARGS(&initialize)))) return 7;
    factory->Release();
    if (FAILED(initialize->QueryInterface(IID_PPV_ARGS(&thumbnail)))) return 8;
    if (FAILED(initialize->QueryInterface(IID_PPV_ARGS(&preview)))) return 9;
    if (FAILED(SHCreateStreamOnFileEx(argv[2], STGM_READ | STGM_SHARE_DENY_NONE, 0, FALSE, nullptr, &stream))) return 10;
    if (FAILED(initialize->Initialize(stream, STGM_READ))) return 11;
    stream->Release();
    HBITMAP bitmap = nullptr;
    WTS_ALPHATYPE alpha = WTSAT_UNKNOWN;
    if (FAILED(thumbnail->GetThumbnail(128, &bitmap, &alpha)) || !bitmap || alpha != WTSAT_ARGB) return 12;
    BITMAP info = {};
    if (!GetObjectW(bitmap, sizeof(info), &info) || info.bmWidth < 1 || info.bmWidth > 128 || info.bmHeight < 1 || info.bmHeight > 128) return 13;
    DeleteObject(bitmap);
    HWND parent = CreateWindowExW(0, L"STATIC", L"preview smoke", WS_OVERLAPPEDWINDOW, 0, 0, 640, 480, nullptr, nullptr, GetModuleHandleW(nullptr), nullptr);
    RECT bounds = {0, 0, 640, 480};
    if (!parent || FAILED(preview->SetWindow(parent, &bounds)) || FAILED(preview->DoPreview())) return 14;
    IOleWindow *ole = nullptr;
    HWND child = nullptr;
    if (FAILED(preview->QueryInterface(IID_PPV_ARGS(&ole))) || FAILED(ole->GetWindow(&child)) || !IsWindow(child)) return 15;
    ole->Release();
    SendMessageW(child, WM_PAINT, 0, 0);
    if (FAILED(preview->Unload()) || IsWindow(child)) return 16;
    preview->Release(); thumbnail->Release(); initialize->Release();
    DestroyWindow(parent);
    // The decoder DLL remains resident by contract until this isolated process exits.
    CoUninitialize();
    std::puts("COM thumbnail, preview window, and resource cleanup passed without registration.");
    return 0;
}
