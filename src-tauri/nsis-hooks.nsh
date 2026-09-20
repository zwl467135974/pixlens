; Tauri NSIS 安装钩子（doc/05 M7：卸载清理缓存目录）
!macro NSIS_HOOK_PREINSTALL
!macroend

!macro NSIS_HOOK_POSTINSTALL
!macroend

!macro NSIS_HOOK_PREUNINSTALL
!macroend

!macro NSIS_HOOK_POSTUNINSTALL
  ; 删除缩略图/预览磁盘缓存与设置（%LOCALAPPDATA%\com.pixlens.app）
  RMDir /r "$LOCALAPPDATA\com.pixlens.app"
  RMDir /r "$APPDATA\com.pixlens.app"
!macroend
