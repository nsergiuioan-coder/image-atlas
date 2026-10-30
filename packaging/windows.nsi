!include "MUI2.nsh"
!include "LogicLib.nsh"
!ifndef VERSION
!define VERSION "0.1.0"
!endif
Name "Image Atlas"
OutFile "..\dist\ImageAtlas-${VERSION}-setup.exe"
InstallDir "$LOCALAPPDATA\Programs\ImageAtlas"
InstallDirRegKey HKCU "Software\NIS\ImageAtlas" "InstallDir"
RequestExecutionLevel user
Unicode True
SetCompressor /SOLID lzma
!define MUI_ICON "..\assets\app.ico"
!define MUI_UNICON "..\assets\app.ico"
!define MUI_ABORTWARNING
!insertmacro MUI_PAGE_WELCOME
!insertmacro MUI_PAGE_LICENSE "..\LICENSE"
!insertmacro MUI_PAGE_DIRECTORY
!insertmacro MUI_PAGE_INSTFILES
!define MUI_FINISHPAGE_RUN "$INSTDIR\ImageAtlas.exe"
!insertmacro MUI_PAGE_FINISH
!insertmacro MUI_UNPAGE_CONFIRM
!insertmacro MUI_UNPAGE_INSTFILES
!insertmacro MUI_LANGUAGE "English"
Section "Image Atlas"
SetShellVarContext current
SetOutPath "$INSTDIR"
File /oname=ImageAtlas.exe "..\target\release\image-atlas.exe"
File "..\LICENSE"
File "..\NOTICE"
File "..\ATTRIBUZIONI.txt"
File "..\README.md"
File /r "..\THIRD-PARTY-LICENSES"
WriteUninstaller "$INSTDIR\Uninstall.exe"
CreateDirectory "$SMPROGRAMS\Image Atlas"
CreateShortcut "$SMPROGRAMS\Image Atlas\Image Atlas.lnk" "$INSTDIR\ImageAtlas.exe"
CreateShortcut "$SMPROGRAMS\Image Atlas\Uninstall.lnk" "$INSTDIR\Uninstall.exe"
WriteRegStr HKCU "Software\NIS\ImageAtlas" "InstallDir" "$INSTDIR"
WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageAtlas" "DisplayName" "Image Atlas"
WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageAtlas" "DisplayVersion" "${VERSION}"
WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageAtlas" "Publisher" "NIS"
WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageAtlas" "UninstallString" '"$INSTDIR\Uninstall.exe"'
WriteRegStr HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageAtlas" "DisplayIcon" "$INSTDIR\ImageAtlas.exe"
WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageAtlas" "NoModify" 1
WriteRegDWORD HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageAtlas" "NoRepair" 1
SectionEnd
Section "Uninstall"
SetShellVarContext current
Delete "$INSTDIR\ImageAtlas.exe"
Delete "$INSTDIR\LICENSE"
Delete "$INSTDIR\NOTICE"
Delete "$INSTDIR\ATTRIBUZIONI.txt"
Delete "$INSTDIR\README.md"
RMDir /r "$INSTDIR\THIRD-PARTY-LICENSES"
Delete "$INSTDIR\Uninstall.exe"
Delete "$SMPROGRAMS\Image Atlas\Image Atlas.lnk"
Delete "$SMPROGRAMS\Image Atlas\Uninstall.lnk"
RMDir "$SMPROGRAMS\Image Atlas"
RMDir "$INSTDIR"
DeleteRegKey HKCU "Software\Microsoft\Windows\CurrentVersion\Uninstall\ImageAtlas"
DeleteRegKey HKCU "Software\NIS\ImageAtlas"
SectionEnd
