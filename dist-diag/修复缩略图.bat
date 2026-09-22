@echo off
rem PixLens thumbnail diagnose + fix (double-click to run)
rem Report: Desktop\PixLens-thumb-diag.txt
cd /d "%~dp0"
echo Running PixLens thumbnail diagnose + fix ...
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0diag.ps1" -Fix
echo.
pause
