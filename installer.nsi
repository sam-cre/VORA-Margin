; ============================================================================
;  VORA-Margin - Windows installer (NSIS 3.x / Modern UI 2)
;  Styled to match the VORA-Stream installer: stock NSIS graphics, the page
;  flow Welcome -> License -> Choose Location -> Installing -> Finish, with a
;  "Create Desktop Shortcut" checkbox on the Finish page.
;  Build with:
;     "C:\Program Files (x86)\NSIS\makensis.exe" installer.nsi
;  Expects the release binary at: target\release\vora-margin.exe
; ============================================================================

Unicode true

; ---- Product metadata ------------------------------------------------------
!define PRODUCT_NAME      "VORA-Margin"
!define PRODUCT_VERSION   "1.0.0"
!define PRODUCT_PUBLISHER "VORA"
!define APP_EXE           "vora-margin.exe"
!define UNINST_KEY        "Software\Microsoft\Windows\CurrentVersion\Uninstall\${PRODUCT_NAME}"
!define APPDATA_DIR       "VORA\Margin"   ; %APPDATA%\VORA\Margin (user pricing data)

!include "MUI2.nsh"
!include "x64.nsh"
!include "LogicLib.nsh"
!include "FileFunc.nsh"

Name "${PRODUCT_NAME}"
OutFile "VORA-Margin-Setup.exe"
InstallDir "$PROGRAMFILES64\VORA-Margin"
InstallDirRegKey HKLM "Software\VORA\Margin" "InstallDir"
RequestExecutionLevel admin
SetCompressor /SOLID lzma

; ---- Setup .exe file properties --------------------------------------------
VIProductVersion "1.0.0.0"
VIAddVersionKey "ProductName"     "${PRODUCT_NAME}"
VIAddVersionKey "FileDescription" "${PRODUCT_NAME} Setup"
VIAddVersionKey "FileVersion"     "${PRODUCT_VERSION}"
VIAddVersionKey "ProductVersion"  "${PRODUCT_VERSION}"
VIAddVersionKey "CompanyName"     "${PRODUCT_PUBLISHER}"
VIAddVersionKey "LegalCopyright"  "(c) 2026 Sam Rogers / ${PRODUCT_PUBLISHER}"

; ---- Modern UI (stock graphics, like the VORA-Stream installer) -------------
!define MUI_ABORTWARNING

; Welcome page
!define MUI_WELCOMEPAGE_TITLE "Welcome to ${PRODUCT_NAME} Setup"
!define MUI_WELCOMEPAGE_TEXT "VORA-Margin is a local pricing calculator for clothing brands - see your real margin after every platform fee.$\r$\n$\r$\nThis installer will:$\r$\n    - Install ${PRODUCT_NAME} on your system$\r$\n    - Create a Start Menu shortcut$\r$\n$\r$\nClick Next to continue."

; Finish page: app installed, with an optional Desktop shortcut (checked)
!define MUI_FINISHPAGE_TITLE "Completing ${PRODUCT_NAME} Setup"
!define MUI_FINISHPAGE_TEXT "${PRODUCT_NAME} has been installed on your computer.$\r$\n$\r$\nClick Finish to close Setup."
!define MUI_FINISHPAGE_SHOWREADME ""
!define MUI_FINISHPAGE_SHOWREADME_TEXT "Create Desktop Shortcut"
!define MUI_FINISHPAGE_SHOWREADME_FUNCTION CreateDesktopShortcut

!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "EULA.txt"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!insertmacro MUI_PAGE_FINISH

!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES

!insertmacro MUI_LANGUAGE "English"

; Creates the optional Desktop shortcut from the Finish-page checkbox.
Function CreateDesktopShortcut
  CreateShortcut "$DESKTOP\${PRODUCT_NAME}.lnk" "$INSTDIR\${APP_EXE}"
FunctionEnd

Function .onInit
  ${IfNot} ${RunningX64}
    MessageBox MB_OK|MB_ICONSTOP "VORA-Margin requires a 64-bit version of Windows."
    Abort
  ${EndIf}
FunctionEnd

; ============================================================================
;  Install
; ============================================================================
Section "Install"
  SetOutPath "$INSTDIR"
  File "target\release\${APP_EXE}"
  File "EULA.txt"
  File "README.md"

  ; Start Menu shortcut (current user)
  CreateDirectory "$SMPROGRAMS\${PRODUCT_NAME}"
  CreateShortcut "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk" "$INSTDIR\${APP_EXE}"

  ; Remember install location + write the uninstaller
  WriteRegStr HKLM "Software\VORA\Margin" "InstallDir" "$INSTDIR"
  WriteUninstaller "$INSTDIR\Uninstall.exe"

  ; Add/Remove Programs entry
  WriteRegStr   HKLM "${UNINST_KEY}" "DisplayName"     "${PRODUCT_NAME}"
  WriteRegStr   HKLM "${UNINST_KEY}" "DisplayVersion"  "${PRODUCT_VERSION}"
  WriteRegStr   HKLM "${UNINST_KEY}" "Publisher"       "${PRODUCT_PUBLISHER}"
  WriteRegStr   HKLM "${UNINST_KEY}" "DisplayIcon"     "$INSTDIR\${APP_EXE},0"
  WriteRegStr   HKLM "${UNINST_KEY}" "InstallLocation" "$INSTDIR"
  WriteRegStr   HKLM "${UNINST_KEY}" "UninstallString" '"$INSTDIR\Uninstall.exe"'
  WriteRegStr   HKLM "${UNINST_KEY}" "QuietUninstallString" '"$INSTDIR\Uninstall.exe" /S'
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoModify" 1
  WriteRegDWORD HKLM "${UNINST_KEY}" "NoRepair" 1

  ; Estimated size for Add/Remove Programs
  ${GetSize} "$INSTDIR" "/S=0K" $0 $1 $2
  IntFmt $0 "0x%08X" $0
  WriteRegDWORD HKLM "${UNINST_KEY}" "EstimatedSize" "$0"
SectionEnd

; ============================================================================
;  Uninstall
; ============================================================================
Section "Uninstall"
  Delete "$INSTDIR\${APP_EXE}"
  Delete "$INSTDIR\EULA.txt"
  Delete "$INSTDIR\README.md"
  Delete "$INSTDIR\Uninstall.exe"
  RMDir  "$INSTDIR"

  ; Shortcuts
  Delete "$SMPROGRAMS\${PRODUCT_NAME}\${PRODUCT_NAME}.lnk"
  RMDir  "$SMPROGRAMS\${PRODUCT_NAME}"
  Delete "$DESKTOP\${PRODUCT_NAME}.lnk"

  ; Registry
  DeleteRegKey HKLM "${UNINST_KEY}"
  DeleteRegKey HKLM "Software\VORA\Margin"

  ; Offer to remove the user's saved pricing data
  ${If} ${FileExists} "$APPDATA\${APPDATA_DIR}\*.*"
    MessageBox MB_YESNO|MB_ICONQUESTION "Also delete your saved VORA-Margin pricing data?$\r$\n$\r$\n$APPDATA\${APPDATA_DIR}$\r$\n$\r$\nChoose No to keep it for a future reinstall." IDNO keepdata
      RMDir /r "$APPDATA\${APPDATA_DIR}"
      RMDir "$APPDATA\VORA"
    keepdata:
  ${EndIf}
SectionEnd
