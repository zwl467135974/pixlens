// probe: print DLL-side GUID bytes + raw CoCreateInstance/QI
#define INITGUID
#include <windows.h>
#include <shobjidl.h>
#include <cstdio>
#pragma comment(lib, "ole32.lib")
#pragma comment(lib, "uuid.lib")

int main() {
    printf("exe-side IID_IInitializeWithStream = ");
    const GUID* g = &IID_IInitializeWithStream;
    for (int i = 0; i < 16; i++) printf("%02X", ((const unsigned char*)g)[i]);
    printf("\n");
    const CLSID clsid = { 0x2B2E7C27, 0xBC52, 0x4521, {0x9A,0x56,0x87,0xBC,0x2D,0xFC,0x76,0x39} };
    CoInitializeEx(nullptr, COINIT_MULTITHREADED);
    IUnknown* unk = nullptr;
    HRESULT hr = CoCreateInstance(clsid, nullptr, CLSCTX_INPROC_SERVER, IID_IUnknown, (void**)&unk);
    printf("CoCreate hr=0x%08X\n", (unsigned)hr);
    if (FAILED(hr)) return 1;
    void* p = nullptr;
    hr = unk->QueryInterface(IID_IInitializeWithStream, &p);
    printf("QI Stream hr=0x%08X\n", (unsigned)hr);
    if (p) ((IUnknown*)p)->Release();
    unk->Release();
    return 0;
}
