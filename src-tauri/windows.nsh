!macro NSIS_HOOK_POSTUNINSTALL
  ${If} $DeleteAppDataCheckboxState = 1
  ${AndIf} $UpdateMode <> 1
    StrCpy $R6 100
    risunest_remove_data:
    ClearErrors
    RmDir /r "$LOCALAPPDATA\RisuNestData"
    RmDir /r "$LOCALAPPDATA\RisuNestWebViewData"
    IfErrors risunest_remove_data_retry risunest_remove_data_done
    risunest_remove_data_retry:
    Sleep 100
    IntOp $R6 $R6 - 1
    IntCmp $R6 0 risunest_remove_data_failed risunest_remove_data_failed risunest_remove_data
    risunest_remove_data_failed:
    DetailPrint "RisuNest application data could not be fully removed."
    SetErrorLevel 1
    risunest_remove_data_done:
  ${EndIf}
!macroend
