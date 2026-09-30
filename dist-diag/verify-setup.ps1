param()
Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing
Add-Type -TypeDefinition @"
using System;
using System.Runtime.InteropServices;
public class Win32Setup {
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
    [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
}
"@
# kill the app first (same window title match)
Get-Process pixlens -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 500

Start-Process -FilePath 'D:\work\pixlens\target\release\bundle\nsis\PixLens_1.3.3_x64-setup.exe'
Start-Sleep -Milliseconds 4000

function FindSetupWindow {
    $procs = Get-Process | Where-Object { $_.MainWindowTitle -like "*PixLens*" -and $_.MainWindowHandle -ne 0 }
    if ($procs) { return $procs[0] } else { return $null }
}
function Shot([IntPtr]$h, [string]$name) {
    $r = New-Object Win32Setup+RECT
    [Win32Setup]::GetWindowRect($h, [ref]$r) | Out-Null
    $w = $r.R - $r.L; $ht = $r.B - $r.T
    if ($w -lt 50) { Write-Output ("BAD_RECT $name ${w}x${ht}"); return }
    $screendc = [Win32Setup]::GetDC([IntPtr]::Zero)
    $memdc = [Win32Setup]::CreateCompatibleDC($screendc)
    $hbm = [Win32Setup]::CreateCompatibleBitmap($screendc, $w, $ht)
    [Win32Setup]::SelectObject($memdc, $hbm) | Out-Null
    [Win32Setup]::PrintWindow($h, $memdc, 2) | Out-Null
    $bmp = [System.Drawing.Image]::FromHbitmap($hbm)
    $bmp.Save("C:\Users\GA\AppData\Local\Temp\setup_$name.png", [System.Drawing.Imaging.ImageFormat]::Png)
    $bmp.Dispose()
    [Win32Setup]::DeleteObject($hbm) | Out-Null
    [Win32Setup]::DeleteDC($memdc) | Out-Null
    [Win32Setup]::ReleaseDC([IntPtr]::Zero, $screendc) | Out-Null
    Write-Output ("SHOT $name ${w}x${ht}")
}

$p = FindSetupWindow
if (-not $p) { Write-Output "NO_SETUP_WINDOW"; exit 1 }
$h = $p.MainWindowHandle
[Win32Setup]::SetForegroundWindow($h) | Out-Null
Start-Sleep -Milliseconds 800
Shot $h "p0_welcome"
[System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
Start-Sleep -Milliseconds 1200
$p2 = FindSetupWindow; if ($p2) { $h = $p2.MainWindowHandle; [Win32Setup]::SetForegroundWindow($h) | Out-Null; Shot $h "p1_mode" }
[System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
Start-Sleep -Milliseconds 1200
$p3 = FindSetupWindow; if ($p3) { $h = $p3.MainWindowHandle; [Win32Setup]::SetForegroundWindow($h) | Out-Null; Shot $h "p2_installing" }
# wait for install to finish (finish page), up to 40s
$finish = $false
for ($i = 0; $i -lt 20; $i++) {
    Start-Sleep -Milliseconds 2000
    $p4 = FindSetupWindow
    if ($p4) {
        $h = $p4.MainWindowHandle
        [Win32Setup]::SetForegroundWindow($h) | Out-Null
        Shot $h ("p3_wait" + $i)
    }
}
Write-Output "DONE"
