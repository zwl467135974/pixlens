param(
    [string]$OutPng = "C:\Users\GA\AppData\Local\Temp\pixlens_ui_shot.png",
    [int]$EscFirst = 1
)
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class Win32Shot {
    [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
    [DllImport("user32.dll")] public static extern bool ShowWindow(IntPtr h, int cmd);
    [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
    [DllImport("user32.dll")] public static extern bool PrintWindow(IntPtr h, IntPtr dc, uint flags);
    [DllImport("user32.dll")] public static extern IntPtr GetDC(IntPtr h);
    [DllImport("user32.dll")] public static extern int ReleaseDC(IntPtr h, IntPtr dc);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleDC(IntPtr dc);
    [DllImport("gdi32.dll")] public static extern IntPtr CreateCompatibleBitmap(IntPtr dc, int w, int h);
    [DllImport("gdi32.dll")] public static extern IntPtr SelectObject(IntPtr dc, IntPtr obj);
    [DllImport("gdi32.dll")] public static extern bool DeleteObject(IntPtr obj);
    [DllImport("gdi32.dll")] public static extern bool DeleteDC(IntPtr dc);
    [DllImport("gdi32.dll")] public static extern bool BitBlt(IntPtr d, int x, int y, int w, int h, IntPtr s, int sx, int sy, int rop);
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
Start-Sleep -Milliseconds 500

$procs = Get-Process | Where-Object { $_.MainWindowTitle -like "*PixLens*" -and $_.MainWindowHandle -ne 0 }
if (-not $procs) { Write-Output "NO_WINDOW"; exit 1 }
$h = $procs[0].MainWindowHandle
[Win32Shot]::ShowWindow($h, 9) | Out-Null   # SW_RESTORE
[Win32Shot]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 700

if ($EscFirst -eq 1) {
    [System.Windows.Forms.SendKeys]::SendWait("{ESC}")
    Start-Sleep -Milliseconds 900
}

$r = New-Object Win32Shot+RECT
[Win32Shot]::GetWindowRect($h, [ref]$r) | Out-Null
$w = $r.R - $r.L; $ht = $r.B - $r.T
if ($w -lt 50 -or $ht -lt 50) { Write-Output "BAD_RECT ${w}x${ht}"; exit 1 }

$screendc = [Win32Shot]::GetDC([IntPtr]::Zero)
$memdc = [Win32Shot]::CreateCompatibleDC($screendc)
$hbm = [Win32Shot]::CreateCompatibleBitmap($screendc, $w, $ht)
[Win32Shot]::SelectObject($memdc, $hbm) | Out-Null
# PW_RENDERFULLCONTENT = 2: capture DirectComposition content (WebView2)
$ok = [Win32Shot]::PrintWindow($h, $memdc, 2)
if (-not $ok) {
    [Win32Shot]::BitBlt($memdc, 0, 0, $w, $ht, $screendc, $r.L, $r.T, 0x00CC0020) | Out-Null
}
$bmp = [System.Drawing.Image]::FromHbitmap($hbm)
$bmp.Save($OutPng, [System.Drawing.Imaging.ImageFormat]::Png)
$bmp.Dispose()
[Win32Shot]::DeleteObject($hbm) | Out-Null
[Win32Shot]::DeleteDC($memdc) | Out-Null
[Win32Shot]::ReleaseDC([IntPtr]::Zero, $screendc) | Out-Null
Write-Output ("SHOT_OK pw={0} {1}x{2} -> {3}" -f $ok, $w, $ht, $OutPng)
