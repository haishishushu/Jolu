; The application uses a stable registry name without spaces.
; Preserve it during upgrades, remove it on explicit uninstall.
!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $UpdateMode <> 1
    DeleteRegValue HKCU "Software\Microsoft\Windows\CurrentVersion\Run" "Jolu"
  ${EndIf}
!macroend
