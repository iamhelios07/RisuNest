!macro NSIS_HOOK_PREUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    ; The template's process check runs after this hook.
    !insertmacro CheckIfAppIsRunning "${MAINBINARYNAME}.exe" "${PRODUCTNAME}"
    ClearErrors
    ExecWait '"$INSTDIR\${MAINBINARYNAME}.exe" --remove-local-data --yes' $R6
    ${If} ${Errors}
    ${OrIf} $R6 != 0
      ; Keep the application and uninstaller available for another attempt.
      SetErrorLevel 1
      Abort "RisuNest application data could not be fully removed. Close RisuNest and retry uninstalling."
    ${EndIf}
  ${EndIf}
!macroend
