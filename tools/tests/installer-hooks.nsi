; Compile-only source harness. This is not the application installer.
Unicode true
Name "VidLib hook verification"
OutFile "${OUT_FILE}"
InstallDir "$LOCALAPPDATA\VidLibHookVerification"
RequestExecutionLevel user
!define MAINBINARYNAME "vidlib-tauri"
!include "../../src-tauri/installer/hooks.nsh"
Section
  SetOutPath "$INSTDIR"
  WriteUninstaller "$INSTDIR\uninstall.exe"
  !insertmacro NSIS_HOOK_POSTINSTALL
SectionEnd
Section "Uninstall"
  !insertmacro NSIS_HOOK_PREUNINSTALL
  Delete "$INSTDIR\uninstall.exe"
  RMDir "$INSTDIR"
SectionEnd
