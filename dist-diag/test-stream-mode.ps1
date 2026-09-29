param()
# Direct activation test: CoCreateInstance(PixLens CLSID) + IInitializeWithStream(file stream)
# -> GetThumbnail. Proves the stream-mode MF path (what Shell binding actually uses).
$ErrorActionPreference = 'Continue'
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
[StructLayout(LayoutKind.Sequential)]
public struct SIZE2 { public int cx; public int cy; }
[ComImport, Guid("2B2E7C27-BC52-4521-9A56-87BC2DFC7639"), ClassInterface(ClassInterfaceType.None)]
public class PixLensThumbCls {}
[ComImport, Guid("e357fccd-a995-4576-b01f-234630154e96"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IThumbnailProvider2 {
    [PreserveSig] int GetThumbnail(uint cx, out IntPtr phbm, out int alpha);
}
[ComImport, Guid("b824b49d-22ac-4161-ac8a-9916e8fa3f7f"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IInitializeWithStream2 {
    [PreserveSig] int Initialize(IStream stm, uint mode);
}
public static class StreamProbe {
    [DllImport("shlwapi.dll", CharSet=CharSet.Unicode)]
    public static extern int SHCreateStreamOnFileW(string path, uint mode, out IStream stm);
    [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr h);
    [DllImport("gdi32.dll")] public static extern int GetObjectW(IntPtr h, int c, byte[] b);
    public static string Probe(string path) {
        IStream stm;
        int hr = SHCreateStreamOnFileW(path, 0, out stm); // STGM_READ
        if (hr != 0) return "STREAM_FAIL hr=0x" + hr.ToString("X8");
        var tp = (IThumbnailProvider2)new PixLensThumbCls();
        var init = (IInitializeWithStream2)tp;
        hr = init.Initialize(stm, 0);
        if (hr != 0) return "INIT_FAIL hr=0x" + hr.ToString("X8");
        IntPtr hbm;
        int alpha;
        hr = tp.GetThumbnail(256, out hbm, out alpha);
        if (hr != 0) return "THUMB_FAIL hr=0x" + hr.ToString("X8");
        var bm = new byte[40];
        int n = GetObjectW(hbm, 40, bm);
        int w = BitConverter.ToInt32(bm, 4), h = BitConverter.ToInt32(bm, 8);
        DeleteObject(hbm);
        return "THUMB_OK " + w + "x" + h;
    }
}
"@
$files = @(Get-ChildItem -LiteralPath "C:\Users\GA\Desktop" -Recurse -Depth 1 | Where-Object { $_.Extension -in ".mp4", ".psd" } | Select-Object -First 3)
foreach ($f in $files) {
    Write-Output ([StreamProbe]::Probe($f.FullName) + "`t" + $f.Extension + "`t" + $f.Name)
}

foreach ($m in [System.Diagnostics.Process]::GetCurrentProcess().Modules) {
    if ($m.ModuleName -like "pixlens*") { Write-Output ("SELFMODULE " + $m.FileName) }
}
