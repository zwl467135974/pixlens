# IShellItemImageFactory 缩略图提取测试（与 Explorer 同链路）
# 成功输出 PNG，即证明注册的缩略图 COM handler 工作正常
param([string]$Path, [string]$OutPng, [int]$Size = 256)
$ErrorActionPreference = "Stop"
Add-Type -AssemblyName System.Drawing
Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class ThumbTest {
    [StructLayout(LayoutKind.Sequential)]
    public struct SIZE { public int cx; public int cy; }

    [ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
    public interface IShellItemImageFactory {
        [PreserveSig] int GetImage(SIZE size, int flags, out IntPtr phbm);
    }

    [DllImport("shell32.dll", CharSet = CharSet.Unicode, PreserveSig = false)]
    static extern void SHCreateItemFromParsingName(string path, IntPtr pbc, ref Guid riid, out IShellItemImageFactory ppv);

    public static int Extract(string path, int size, string outPng) {
        Guid iid = typeof(IShellItemImageFactory).GUID;
        IShellItemImageFactory factory;
        SHCreateItemFromParsingName(path, IntPtr.Zero, ref iid, out factory);
        SIZE s; s.cx = size; s.cy = size;
        IntPtr hbmp;
        int hr = factory.GetImage(s, 0x8, out hbmp); // SIIGBF_THUMBNAILONLY
        if (hr != 0) return hr;
        using (var img = System.Drawing.Image.FromHbitmap(hbmp)) {
            img.Save(outPng, System.Drawing.Imaging.ImageFormat.Png);
        }
        return 0;
    }
}
"@

try {
    [int]$hr = [ThumbTest]::Extract($Path, $Size, $OutPng)
    if ($hr -eq 0) {
        Write-Output "OK -> $OutPng"
    } else {
        Write-Output ("FAIL hr=0x{0:X}" -f $hr)
        exit 1
    }
} catch {
    Write-Output "EXCEPTION: $($_.Exception.Message)"
    exit 1
}
