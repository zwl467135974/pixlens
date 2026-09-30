@echo off
rem PixLens video thumbnail test - DRAG an mp4 file onto THIS icon
rem Result line to look for: "MP4  GetImage hr = 0x00000000  SUCCESS"
cd /d "%~dp0"
if "%~1"=="" (
  echo Please DRAG an mp4/video file ONTO this icon, then release.
  echo 把视频文件拖到本图标上松开即可测试。
  pause
  exit /b
)
"%SystemRoot%\System32\WindowsPowerShell\v1.0\powershell.exe" -NoProfile -ExecutionPolicy Bypass -File "%~dp0diag.ps1" -Mp4 "%~1"
echo.
echo Look for the line:  [6a] ... MP4  GetImage hr = 0x00000000  SUCCESS
pause
