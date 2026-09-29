param()
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
[StructLayout(LayoutKind.Sequential)]
public struct SIZE { public int cx; public int cy; }
[ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IShellItemImageFactory {
    [PreserveSig] int GetImage(SIZE size, int flags, out IntPtr phbm);
}
public static class ThumbProbe2 {
    [DllImport("shell32.dll", CharSet=CharSet.Unicode)]
    private static extern int SHCreateItemFromParsingName(string path, IntPtr pbc, ref Guid riid, out IShellItemImageFactory ppv);
    [DllImport("gdi32.dll")] private static extern bool DeleteObject(IntPtr h);
    public static string Probe(string path) {
        Guid iid = new Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b");
        IShellItemImageFactory fac;
        int hr = SHCreateItemFromParsingName(path, IntPtr.Zero, ref iid, out fac);
        if (hr != 0) return "CREATE_FAIL hr=0x" + hr.ToString("X8");
        SIZE sz; sz.cx = 256; sz.cy = 256;
        IntPtr hbm;
        hr = fac.GetImage(sz, 8, out hbm);
        if (hr != 0) return "GETIMAGE_FAIL hr=0x" + hr.ToString("X8");
        DeleteObject(hbm);
        return "GETIMAGE_OK";
    }
}
"@
# touch to bust cache, then probe, then immediately list dllhost modules
Get-ChildItem -LiteralPath "C:\Users\GA\Desktop" -Recurse -Depth 1 -Filter *.mp4 | Select-Object -First 1 | ForEach-Object { $_.LastWriteTime = Get-Date }
$f = (Get-ChildItem -LiteralPath "C:\Users\GA\Desktop" -Recurse -Depth 1 -Filter *.mp4 | Select-Object -First 1).FullName
Write-Output ("PROBE " + [ThumbProbe2]::Probe($f))
foreach ($p in Get-Process dllhost -ErrorAction SilentlyContinue) {
    try {
        foreach ($m in $p.Modules) {
            if ($m.ModuleName -like "pixlens*") { Write-Output ("DLLHOST " + $p.Id + " " + $m.FileName) }
        }
    } catch {}
}
Write-Output "DONE"
