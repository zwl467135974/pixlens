@echo off
rem Build PixLens PSD/PSB shell thumbnail extension (C++ shell + Rust core)
cd /d "%~dp0.."
call "C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1

cl /nologo /LD /EHsc /O2 /W3 /utf-8 /Zi /DUNICODE /D_UNICODE ^
   shell-thumb-cpp\thumb.cpp ^
   /Fe:target\release\pixlens_thumb_cpp.dll ^
   /Fo:target\release\pixlens_thumb_cpp.obj ^
   /link /LIBPATH:target\release psd_capi.lib gdi32.lib shlwapi.lib ole32.lib user32.lib advapi32.lib ws2_32.lib bcrypt.lib ntdll.lib userenv.lib synchronization.lib

if exist target\release\pixlens_thumb_cpp.dll (echo BUILD_OK) else (echo BUILD_FAILED)
