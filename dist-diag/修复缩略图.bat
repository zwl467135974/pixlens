@echo off
rem PixLens thumbnail diagnose + fix (double-click to run)
rem Report: Desktop\PixLens-thumb-diag.txt
rem Force 64-bit PowerShell so registry view matches Explorer
cd /d "%~dp0"
echo Running PixLens thumbnail diagnose + fix ...
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0diag.ps1" -Fix
echo.
pause
