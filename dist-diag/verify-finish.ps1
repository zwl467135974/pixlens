param()
Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WinEnum2 {
    public delegate bool EnumProc(IntPtr h, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc cb, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder sb, int max);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder sb, int max);
}
"@
Start-Process -FilePath 'D:\work\pixlens\target\release\bundle\nsis\PixLens_1.3.3_x64-setup.exe'
# poll for window up to 20s
$hwnd = [IntPtr]::Zero
for ($i = 0; $i -lt 40; $i++) {
    Start-Sleep -Milliseconds 500
    $p = Get-Process | Where-Object { $_.MainWindowTitle -like "*PixLens*" -and $_.MainWindowHandle -ne 0 }
    if ($p) { $hwnd = $p[0].MainWindowHandle; break }
}
if ($hwnd -eq [IntPtr]::Zero) { Write-Output "NO_WINDOW"; exit 1 }
Write-Output "WINDOW_UP"
Add-Type -AssemblyName System.Windows.Forms
# press Enter through: welcome -> mode -> (install runs) -> finish; check texts after each
for ($step = 0; $step -lt 6; $step++) {
    $p = Get-Process | Where-Object { $_.MainWindowTitle -like "*PixLens*" -and $_.MainWindowHandle -ne 0 }
    if (-not $p) { Write-Output "WINDOW_CLOSED at step $step"; break }
    $h = $p[0].MainWindowHandle
    $texts = New-Object System.Collections.ArrayList
    $cb = [WinEnum2+EnumProc]{ param($hh, $lp)
        $sb = New-Object System.Text.StringBuilder 1024
        [void][WinEnum2]::GetWindowTextW($hh, $sb, 1024)
        if ($sb.Length -gt 3) { [void]$texts.Add($sb.ToString()) }
        return $true
    }
    [void][WinEnum2]::EnumChildWindows($h, $cb, [IntPtr]::Zero)
    $joined = $texts -join " // "
    Write-Output ("STEP" + $step + ": " + $joined.Substring(0, [Math]::Min(260, $joined.Length)))
    if ($joined -like "*资源管理器*") { Write-Output "FINISH_TEXT_FOUND"; break }
    [System.Windows.Forms.SendKeys]::SendWait("{ENTER}")
    Start-Sleep -Milliseconds 2500
}
