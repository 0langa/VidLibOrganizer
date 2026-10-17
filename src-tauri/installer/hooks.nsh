; Optional per-user Explorer verb. No shell, script runtime, or apply argument.
; Document model opens each selected item in a separate inactive review window.
; Silent installs do not opt the user in. Existing foreign registrations survive.
!macro VidLibRegisterVerb CLASS LABEL
  ReadRegStr $0 HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" "VidLibOrganizerOwner"
  StrCmp $0 "$INSTDIR\${MAINBINARYNAME}.exe" owned_${LABEL}
  StrCmp $0 "" 0 done_${LABEL}
  ClearErrors
  EnumRegValue $1 HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" 0
  IfErrors 0 done_${LABEL}
  ClearErrors
  EnumRegKey $1 HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" 0
  IfErrors write_${LABEL} done_${LABEL}
owned_${LABEL}:
  ReadRegStr $0 HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename\command" ""
  StrCmp $0 '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --select -- $\"%1$\"' write_${LABEL} done_${LABEL}
write_${LABEL}:
  WriteRegStr HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" "" "Review rename in VidLibOrganizer"
  WriteRegStr HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" "VidLibOrganizerOwner" "$INSTDIR\${MAINBINARYNAME}.exe"
  WriteRegStr HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" "MultiSelectModel" "Document"
  WriteRegStr HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename\command" "" '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --select -- $\"%1$\"'
done_${LABEL}:
!macroend

!macro VidLibRemoveVerb CLASS LABEL
  ReadRegStr $0 HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" "VidLibOrganizerOwner"
  StrCmp $0 "$INSTDIR\${MAINBINARYNAME}.exe" 0 done_${LABEL}
  ReadRegStr $0 HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename\command" ""
  StrCmp $0 '$\"$INSTDIR\${MAINBINARYNAME}.exe$\" --select -- $\"%1$\"' 0 done_${LABEL}
  DeleteRegValue HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename\command" ""
  DeleteRegKey /ifempty HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename\command"
  DeleteRegValue HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" "VidLibOrganizerOwner"
  ReadRegStr $0 HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" ""
  StrCmp $0 "Review rename in VidLibOrganizer" 0 +2
  DeleteRegValue HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" ""
  ReadRegStr $0 HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" "MultiSelectModel"
  StrCmp $0 "Document" 0 +2
  DeleteRegValue HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename" "MultiSelectModel"
  DeleteRegKey /ifempty HKCU "Software\Classes\${CLASS}\shell\VidLibOrganizer.Rename"
done_${LABEL}:
!macroend

!macro NSIS_HOOK_POSTINSTALL
  IfSilent vidlib_optin_done
  MessageBox MB_YESNO|MB_ICONQUESTION|MB_DEFBUTTON2 "Add an optional Explorer action to review selected files or folders? Each selected item opens a separate review window. Nothing is renamed until you preview and confirm." IDNO vidlib_optin_done
  !insertmacro VidLibRegisterVerb "*" files_install
  !insertmacro VidLibRegisterVerb "Directory" folders_install
vidlib_optin_done:
!macroend

!macro NSIS_HOOK_PREUNINSTALL
  !insertmacro VidLibRemoveVerb "*" files_uninstall
  !insertmacro VidLibRemoveVerb "Directory" folders_uninstall
!macroend
