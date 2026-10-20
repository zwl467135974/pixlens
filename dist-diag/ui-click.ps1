# Click at client-area coordinates of a process main window (ASCII only).
# Usage: ui-click.ps1 -X 1071 -Y 24 [-Proc pixlens]
param(
  [int]$X = 0,
  [int]$Y = 0,
  [string]$Proc = "pixlens"
)
Add-Type @"
using System;
using System.Runtime.InteropServices;
public class ClickWin {
  [StructLayout(LayoutKind.Sequential)] public struct RECT { public int L, T, R, B; }
  [DllImport("user32.dll")] public static extern bool SetForegroundWindow(IntPtr h);
  [DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
  [DllImport("user32.dll")] public static extern bool SetCursorPos(int x, int y);
  [DllImport("user32.dll")] public static extern void mouse_event(uint f, uint dx, uint dy, uint d, UIntPtr e);
}
"@
$p = Get-Process $Proc -ErrorAction Stop | Where-Object { $_.MainWindowHandle -ne 0 } | Select-Object -First 1
if (-not $p) { Write-Host "NO_WINDOW"; exit 1 }
$h = $p.MainWindowHandle
[void][ClickWin]::SetForegroundWindow($h)
Start-Sleep -Milliseconds 400
$r = New-Object ClickWin+RECT
[void][ClickWin]::GetWindowRect($h, [ref]$r)
# Tauri frameless window: window rect ~= client rect (DPI 100% assumed; pass -DX/-DY to adjust)
$sx = $r.L + $X
$sy = $r.T + $Y
[void][ClickWin]::SetCursorPos($sx, $sy)
Start-Sleep -Milliseconds 150
[ClickWin]::mouse_event(0x0002, 0, 0, 0, [UIntPtr]::Zero)  # LEFTDOWN
[ClickWin]::mouse_event(0x0004, 0, 0, 0, [UIntPtr]::Zero)  # LEFTUP
Write-Host ("CLICKED client({0},{1}) screen({2},{3}) winRect L{4} T{5} R{6} B{7}" -f $X, $Y, $sx, $sy, $r.L, $r.T, $r.R, $r.B)
