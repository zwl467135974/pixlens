@echo off
rem Build PixLens PSD PSB shell thumbnail extension C++ shell + Rust cdylib
cd /d "%~dp0.."
call "C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1

del target\release\pixlens_thumb_cpp.dll 2>nul
del target\release\pixlens_thumb_cpp.obj 2>nul

cl /nologo /LD /EHsc /O2 /W3 /utf-8 /Zi /MT /DUNICODE /D_UNICODE ^
   shell-thumb-cpp\thumb.cpp ^
   /Fe:target\release\pixlens_thumb_cpp.dll ^
   /Fo:target\release\pixlens_thumb_cpp.obj ^
   /link /DEF:shell-thumb-cpp\exports.def /DEBUG gdi32.lib shlwapi.lib ole32.lib user32.lib

if exist target\release\pixlens_thumb_cpp.dll (echo BUILD_OK) else (echo BUILD_FAILED)
