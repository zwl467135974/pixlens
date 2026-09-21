; Tauri NSIS 安装钩子（doc/05 M7：卸载清理缓存目录；注册 PSD/PSB Explorer 缩略图 COM）
!macro NSIS_HOOK_PREINSTALL
  ; 释放被占用的文件：运行中的主程序，以及加载了缩略图 DLL 的 COM 代理进程
  ; （DllHost 是无状态代理，Windows 会在下次请求时自动重启，可安全结束）
  nsExec::ExecToLog 'taskkill /f /im pixlens.exe'
  nsExec::ExecToLog 'taskkill /f /im dllhost.exe'
  Pop $0
  Sleep 500
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; ── PSD/PSB 资源管理器缩略图 COM 注册（per-user） ──
  ; CLSID {2B2E7C27-BC52-4521-9A56-87BC2DFC7639}，与 shell-thumb-cpp/thumb.cpp 保持一致
  WriteRegStr HKCU "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}" "" "PixLens PSD PSB Thumbnail"
  WriteRegStr HKCU "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}\InprocServer32" "" "$INSTDIR\pixlens_thumb_cpp.dll"
  WriteRegStr HKCU "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}\InprocServer32" "ThreadingModel" "Apartment"
  WriteRegStr HKCU "Software\Classes\.psd\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  WriteRegStr HKCU "Software\Classes\.psb\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  WriteRegStr HKCU "Software\Classes\PSD 图像\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  WriteRegStr HKCU "Software\Classes\PSB 图像\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 卸载也要先释放：DllHost 持有缩略图 DLL 时文件删不掉
  nsExec::ExecToLog 'taskkill /f /im pixlens.exe'
  nsExec::ExecToLog 'taskkill /f /im dllhost.exe'
  Pop $0
  Sleep 500
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; 删除缩略图 COM 注册
  DeleteRegKey HKCU "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  DeleteRegKey HKCU "Software\Classes\.psd\shellex\{E357FCCD-A995-4576-B01F-234630154E96}"
  DeleteRegKey HKCU "Software\Classes\.psb\shellex\{E357FCCD-A995-4576-B01F-234630154E96}"
  DeleteRegKey HKCU "Software\Classes\PSD 图像\shellex\{E357FCCD-A995-4576-B01F-234630154E96}"
  DeleteRegKey HKCU "Software\Classes\PSB 图像\shellex\{E357FCCD-A995-4576-B01F-234630154E96}"
  ; 删除缩略图/预览磁盘缓存与设置
  RMDir /r "$LOCALAPPDATA\com.pixlens.app"
  RMDir /r "$APPDATA\com.pixlens.app"
!macroend
