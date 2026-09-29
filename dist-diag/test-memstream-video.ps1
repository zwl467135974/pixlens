# Simulate GUID-named stream (no extension): mem-stream wrap of mp4 bytes
$ErrorActionPreference = 'Continue'
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
using System.Runtime.InteropServices.ComTypes;
[ComImport, Guid("2B2E7C27-BC52-4521-9A56-87BC2DFC7639"), ClassInterface(ClassInterfaceType.None)]
public class PixLensThumbCls {}
[ComImport, Guid("e357fccd-a995-4576-b01f-234630154e96"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IThumb3 {
    [PreserveSig] int GetThumbnail(uint cx, out IntPtr hbmp, out int alpha);
}
[ComImport, Guid("b824b49d-22ac-4161-ac8a-9916e8fa3f7f"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IInitStream3 {
    [PreserveSig] int Initialize(IStream stm, uint mode);
}
public static class MemStreamProbe {
    [DllImport("shlwapi.dll")]
    private static extern IStream SHCreateMemStream(byte[] data, uint len);
    [DllImport("gdi32.dll")] private static extern bool DeleteObject(IntPtr h);
    public static string Probe(byte[] data) {
        IStream stm = SHCreateMemStream(data, (uint)data.Length);
        var tp = (IThumb3)new PixLensThumbCls();
        int hr = ((IInitStream3)tp).Initialize(stm, 0);
        if (hr != 0) return "INIT_FAIL hr=0x" + hr.ToString("X8");
        IntPtr hbm;
        int alpha;
        hr = tp.GetThumbnail(256, out hbm, out alpha);
        if (hr != 0) return "THUMB_FAIL hr=0x" + hr.ToString("X8");
        DeleteObject(hbm);
        return "THUMB_OK (GUID-name stream)";
    }
}
"@
$mp4 = Get-ChildItem "C:\Users\GA\Desktop" -Recurse -Depth 1 -Filter *.mp4 | Select-Object -First 1
$bytes = [IO.File]::ReadAllBytes($mp4.FullName)
Write-Output ("MP4 memstream: " + [MemStreamProbe]::Probe($bytes))
$psd = Get-Item "C:\Users\GA\Desktop\PSD*\small_rgb8_rle.psd"
$bytes2 = [IO.File]::ReadAllBytes($psd.FullName)
Write-Output ("PSD memstream: " + [MemStreamProbe]::Probe($bytes2))
