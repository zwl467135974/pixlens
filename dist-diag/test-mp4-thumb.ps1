param(
    [string[]]$Files = @()
)
$ErrorActionPreference = 'Continue'

Add-Type -ReferencedAssemblies System.Drawing -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
[StructLayout(LayoutKind.Sequential)]
public struct SIZE { public int cx; public int cy; }
[ComImport, Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b"), InterfaceType(ComInterfaceType.InterfaceIsIUnknown)]
public interface IShellItemImageFactory {
    [PreserveSig] int GetImage(SIZE size, int flags, out IntPtr phbm);
}
public static class ThumbProbe {
    [DllImport("shell32.dll", CharSet=CharSet.Unicode)]
    private static extern int SHCreateItemFromParsingName(string path, IntPtr pbc, ref Guid riid, out IShellItemImageFactory ppv);
    [DllImport("gdi32.dll")] private static extern bool DeleteObject(IntPtr h);

    public static string Probe(string path, string outPng) {
        Guid iid = new Guid("bcc18b79-ba16-442f-80c4-8a59c30c463b");
        IShellItemImageFactory fac;
        int hr = SHCreateItemFromParsingName(path, IntPtr.Zero, ref iid, out fac);
        if (hr != 0) return "CREATE_FAIL hr=0x" + hr.ToString("X8");
        SIZE sz; sz.cx = 256; sz.cy = 256;
        IntPtr hbm;
        hr = fac.GetImage(sz, 8, out hbm);
        if (hr != 0) return "GETIMAGE_FAIL hr=0x" + hr.ToString("X8");
        try {
            using (var img = System.Drawing.Image.FromHbitmap(hbm)) {
                img.Save(outPng, System.Drawing.Imaging.ImageFormat.Png);
            }
            return "OK";
        } catch (Exception e) {
            return "DECODE_FAIL " + e.Message;
        } finally {
            DeleteObject(hbm);
        }
    }
}
"@
Add-Type -AssemblyName System.Drawing

if ($Files.Count -eq 0) {
    $Files = @(Get-ChildItem -LiteralPath "C:\Users\GA\Desktop" -Recurse -Depth 1 -Filter *.mp4 | ForEach-Object { $_.FullName })
}
foreach ($f in $Files) {
    if (-not (Test-Path -LiteralPath $f)) { Write-Output ("MISSING`t" + $f); continue }
    $out = Join-Path $env:TEMP ("mp4thumb_" + [IO.Path]::GetFileNameWithoutExtension($f) + ".png")
    $r = [ThumbProbe]::Probe($f, $out)
    Write-Output ($r + "`t" + $f + "`t" + $out)
}

# prove which handler DLL served the extraction (in-process load check)
$mods = [System.Diagnostics.Process]::GetCurrentProcess().Modules | Where-Object { $_.ModuleName -like "pixlens*" -or $_.ModuleName -like "mf*" }
foreach ($m in $mods) { Write-Output ("MODULE`t" + $m.ModuleName) }
