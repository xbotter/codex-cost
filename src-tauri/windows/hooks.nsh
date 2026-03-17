!macro NSIS_HOOK_POSTINSTALL
  SetOutPath "$INSTDIR"
  File /nonfatal "..\..\build\webview2-com-sys-*\out\x64\WebView2Loader.dll"
  Delete "$SMPROGRAMS\${PRODUCTNAME}.lnk"
  CreateShortcut "$SMPROGRAMS\${PRODUCTNAME}.lnk" "$INSTDIR\${MAINBINARYNAME}.exe"
!macroend