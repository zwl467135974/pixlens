param()
Add-Type -TypeDefinition @"
using System;
using System.Text;
using System.Runtime.InteropServices;
public class WinEnum {
    public delegate bool EnumProc(IntPtr h, IntPtr lp);
    [DllImport("user32.dll")] public static extern bool EnumChildWindows(IntPtr p, EnumProc cb, IntPtr lp);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, StringBuilder sb, int max);
    [DllImport("user32.dll", CharSet=CharSet.Unicode)] public static extern int GetClassNameW(IntPtr h, StringBuilder sb, int max);
}
"@
$procs = Get-Process | Where-Object { $_.MainWindowTitle -like "*PixLens*" -and $_.MainWindowHandle -ne 0 }
if (-not $procs) { Write-Output "NO_WINDOW (installer closed?)"; exit 1 }
Write-Output ("WINDOW: " + $procs[0].MainWindowTitle + " pid=" + $procs[0].Id)
$hwnd = $procs[0].MainWindowHandle
$texts = New-Object System.Collections.ArrayList
$cb = [WinEnum+EnumProc]{ param($h, $lp)
    $sb = New-Object System.Text.StringBuilder 512
    [void][WinEnum]::GetWindowTextW($h, $sb, 512)
    $cls = New-Object System.Text.StringBuilder 256
    [void][WinEnum]::GetClassNameW($h, $cls, 256)
    if ($sb.Length -gt 0) { [void]$texts.Add($cls.ToString() + " | " + $sb.ToString()) }
    return $true
}
[void][WinEnum]::EnumChildWindows($hwnd, $cb, [IntPtr]::Zero)
foreach ($t in $texts) { Write-Output ("CTL: " + $t) }
