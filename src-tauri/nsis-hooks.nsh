; Tauri NSIS 安装钩子（doc/05 M7：卸载清理缓存目录；注册 PSD/PSB/视频 Explorer 缩略图 COM）
; 注意：NSIS 标签在 section 内全局，宏多次展开必须用 IDX 参数保证标签唯一。
!macro NSIS_HOOK_PREINSTALL
  ; 释放被占用的文件：运行中的主程序，以及加载了缩略图 DLL 的 COM 代理进程
  ; （DllHost 是无状态代理，Windows 会在下次请求时自动重启，可安全结束）
  nsExec::ExecToLog 'taskkill /f /im pixlens.exe'
  nsExec::ExecToLog 'taskkill /f /im dllhost.exe'
  Pop $0
  Sleep 500
!macroend

; ── 视频扩展缩略图接管（v1.3.1）────────────────────────────────
; Explorer 自带的视频缩略图依赖机器解码环境（纯净系统经常出不来，实测无解码
; 套件的机器 mp4 全军覆没）。PixLens 统一接管 mp4/m4v/mov/webm/mkv/avi/wmv
; 的 shellex 缩略图键，原值备份、卸载恢复——不给系统留坑。
!macro PIX_TAKEOVER_VIDEO EXTNAME IDX ROOTKEY BACKUPROOT
  ; 首次安装才备份（升级不覆盖，保留最早原值）；(none) 哨兵 = 原本无 handler
  ReadRegStr $0 ${BACKUPROOT} "Software\PixLens\ThumbBackup" "${EXTNAME}"
  StrCmp $0 "" 0 pix_bk_done_${IDX}
    ReadRegStr $0 HKCR ".${EXTNAME}\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" ""
    StrCmp $0 "" 0 pix_bk_save_${IDX}
      WriteRegStr ${BACKUPROOT} "Software\PixLens\ThumbBackup" "${EXTNAME}" "(none)"
      Goto pix_bk_done_${IDX}
    pix_bk_save_${IDX}:
      WriteRegStr ${BACKUPROOT} "Software\PixLens\ThumbBackup" "${EXTNAME}" "$0"
  pix_bk_done_${IDX}:
  WriteRegStr ${ROOTKEY} "Software\Classes\.${EXTNAME}\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
!macroend

!macro PIX_RESTORE_VIDEO EXTNAME IDX ROOTKEY BACKUPROOT
  ReadRegStr $0 ${BACKUPROOT} "Software\PixLens\ThumbBackup" "${EXTNAME}"
  StrCmp $0 "" pix_rs_del_${IDX}
  StrCmp $0 "(none)" pix_rs_del_${IDX}
    WriteRegStr ${ROOTKEY} "Software\Classes\.${EXTNAME}\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "$0"
    Goto pix_rs_done_${IDX}
  pix_rs_del_${IDX}:
    DeleteRegKey ${ROOTKEY} "Software\Classes\.${EXTNAME}\shellex\{E357FCCD-A995-4576-B01F-234630154E96}"
  pix_rs_done_${IDX}:
    DeleteRegValue ${BACKUPROOT} "Software\PixLens\ThumbBackup" "${EXTNAME}"
!macroend

!macro NSIS_HOOK_POSTINSTALL
  ; ── PSD/PSB/视频 资源管理器缩略图 COM 注册 ──
  ; CLSID {2B2E7C27-BC52-4521-9A56-87BC2DFC7639}，与 shell-thumb-cpp/thumb.cpp 保持一致。
  ; 按安装模式分流（v1.1.4）：实测部分机器的缩略图提取运行于 HKCU 不可见的
  ; SYSTEM 代理上下文，仅 HKCU 时整体 CLASSNOTREG——HKLM 是商业 shell 扩展的
  ; 标准位置。"为所有用户安装"写 HKLM（对所有上下文/账户可见）；
  ; "仅为我安装"写 HKCU（不把机器级注册指向个人目录）。
  StrCmp $MultiUser.InstallMode "AllUsers" 0 pix_reg_hkcu
  WriteRegStr HKLM "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}" "" "PixLens Thumbnail"
  WriteRegStr HKLM "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}\InprocServer32" "" "$INSTDIR\pixlens_thumb_cpp.dll"
  WriteRegStr HKLM "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}\InprocServer32" "ThreadingModel" "Apartment"
  WriteRegStr HKLM "Software\Classes\.psd\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  WriteRegStr HKLM "Software\Classes\.psb\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  !insertmacro PIX_TAKEOVER_VIDEO "mp4"  11 HKLM HKLM
  !insertmacro PIX_TAKEOVER_VIDEO "m4v"  12 HKLM HKLM
  !insertmacro PIX_TAKEOVER_VIDEO "mov"  13 HKLM HKLM
  !insertmacro PIX_TAKEOVER_VIDEO "webm" 14 HKLM HKLM
  !insertmacro PIX_TAKEOVER_VIDEO "mkv"  15 HKLM HKLM
  !insertmacro PIX_TAKEOVER_VIDEO "avi"  16 HKLM HKLM
  !insertmacro PIX_TAKEOVER_VIDEO "wmv"  17 HKLM HKLM
  Goto pix_reg_done
  pix_reg_hkcu:
  WriteRegStr HKCU "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}" "" "PixLens Thumbnail"
  WriteRegStr HKCU "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}\InprocServer32" "" "$INSTDIR\pixlens_thumb_cpp.dll"
  WriteRegStr HKCU "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}\InprocServer32" "ThreadingModel" "Apartment"
  WriteRegStr HKCU "Software\Classes\.psd\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  WriteRegStr HKCU "Software\Classes\.psb\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  WriteRegStr HKCU "Software\Classes\PSD 图像\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  WriteRegStr HKCU "Software\Classes\PSB 图像\shellex\{E357FCCD-A995-4576-B01F-234630154E96}" "" "{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  !insertmacro PIX_TAKEOVER_VIDEO "mp4"  21 HKCU HKCU
  !insertmacro PIX_TAKEOVER_VIDEO "m4v"  22 HKCU HKCU
  !insertmacro PIX_TAKEOVER_VIDEO "mov"  23 HKCU HKCU
  !insertmacro PIX_TAKEOVER_VIDEO "webm" 24 HKCU HKCU
  !insertmacro PIX_TAKEOVER_VIDEO "mkv"  25 HKCU HKCU
  !insertmacro PIX_TAKEOVER_VIDEO "avi"  26 HKCU HKCU
  !insertmacro PIX_TAKEOVER_VIDEO "wmv"  27 HKCU HKCU
  pix_reg_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  ; 卸载也要先释放：DllHost 持有缩略图 DLL 时文件删不掉
  nsExec::ExecToLog 'taskkill /f /im pixlens.exe'
  nsExec::ExecToLog 'taskkill /f /im dllhost.exe'
  Pop $0
  Sleep 500
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; 删除缩略图 COM 注册并恢复视频扩展的原 handler（HKCU + 管理员时一并清理 HKLM）
  DeleteRegKey HKCU "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  DeleteRegKey HKCU "Software\Classes\.psd\shellex\{E357FCCD-A995-4576-B01F-234630154E96}"
  DeleteRegKey HKCU "Software\Classes\.psb\shellex\{E357FCCD-A995-4576-B01F-234630154E96}"
  DeleteRegKey HKCU "Software\Classes\PSD 图像\shellex\{E357FCCD-A995-4576-B01F-234630154E96}"
  DeleteRegKey HKCU "Software\Classes\PSB 图像\shellex\{E357FCCD-A995-4576-B01F-234630154E96}"
  !insertmacro PIX_RESTORE_VIDEO "mp4"  31 HKCU HKCU
  !insertmacro PIX_RESTORE_VIDEO "m4v"  32 HKCU HKCU
  !insertmacro PIX_RESTORE_VIDEO "mov"  33 HKCU HKCU
  !insertmacro PIX_RESTORE_VIDEO "webm" 34 HKCU HKCU
  !insertmacro PIX_RESTORE_VIDEO "mkv"  35 HKCU HKCU
  !insertmacro PIX_RESTORE_VIDEO "avi"  36 HKCU HKCU
  !insertmacro PIX_RESTORE_VIDEO "wmv"  37 HKCU HKCU
  UserInfo::GetAccountType
  Pop $0
  StrCmp $0 "Admin" 0 pix_unhklm_skip
  DeleteRegKey HKLM "Software\Classes\CLSID\{2B2E7C27-BC52-4521-9A56-87BC2DFC7639}"
  DeleteRegKey HKLM "Software\Classes\.psd\shellex\{E357FCCD-A995-4576-B01F-234630154E96}"
  DeleteRegKey HKLM "Software\Classes\.psb\shellex\{E357FCCD-A995-4576-B01F-234630154E96}"
  !insertmacro PIX_RESTORE_VIDEO "mp4"  41 HKLM HKLM
  !insertmacro PIX_RESTORE_VIDEO "m4v"  42 HKLM HKLM
  !insertmacro PIX_RESTORE_VIDEO "mov"  43 HKLM HKLM
  !insertmacro PIX_RESTORE_VIDEO "webm" 44 HKLM HKLM
  !insertmacro PIX_RESTORE_VIDEO "mkv"  45 HKLM HKLM
  !insertmacro PIX_RESTORE_VIDEO "avi"  46 HKLM HKLM
  !insertmacro PIX_RESTORE_VIDEO "wmv"  47 HKLM HKLM
  pix_unhklm_skip:
  DeleteRegKey HKCU "Software\PixLens\ThumbBackup"
  DeleteRegKey HKLM "Software\PixLens\ThumbBackup"
  ; 删除缩略图/预览磁盘缓存与设置
  RMDir /r "$LOCALAPPDATA\com.pixlens.app"
  RMDir /r "$APPDATA\com.pixlens.app"
!macroend
