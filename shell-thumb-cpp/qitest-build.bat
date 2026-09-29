@echo off
cd /d "%~dp0.."
call "C:\Program Files (x86)\Microsoft Visual Studio\2019\BuildTools\VC\Auxiliary\Build\vcvars64.bat" >nul 2>&1
cl /nologo /EHsc /W3 /utf-8 /MT shell-thumb-cpp\qitest.cpp /Fe:target\release\qitest.exe
if exist target\release\qitest.exe (echo QITEST_BUILD_OK) else (echo QITEST_BUILD_FAILED)
