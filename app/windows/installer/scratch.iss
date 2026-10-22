; Scratch for Windows: per-user Inno Setup installer, no admin rights.
;
; Built in CI by .github/workflows/windows-build.yml after `flutter build windows`:
;   iscc /DAppVersion=0.2.0 /DSha7=abc1234 /DSourceDir=<Release folder> /DOutputDir=<dir> scratch.iss
; Output: Scratch-Setup-<AppVersion>-<Sha7>.exe
; Optional /DVersionText=<text> (e.g. 0.1.0-prealpha.1) is the displayed version;
; AppVersion stays numeric for the file version.
;
; Installs to %LOCALAPPDATA%\Programs\Scratch. The exe stays notes.exe; the
; Start menu shortcut, desktop shortcut and Apps & features entry say Scratch.
; User data in %LOCALAPPDATA%\Notes is never touched. See wiki/architecture/windows-install.md.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef VersionText
  #define VersionText AppVersion
#endif
#ifndef Sha7
  #define Sha7 "local"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\build\windows\x64\runner\Release"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\build\windows\installer"
#endif

#define AppName "Scratch"
#define AppExe "notes.exe"

[Setup]
; Fixed AppId: every later installer with this id upgrades the same install in place.
AppId={{1B472506-0E7C-46CD-B530-F256FB709BB5}
AppName={#AppName}
AppVersion={#VersionText}
AppVerName={#AppName} {#VersionText}
AppPublisher=BubbleBuffer
VersionInfoVersion={#AppVersion}
VersionInfoProductName={#AppName}
VersionInfoCompany=BubbleBuffer
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\Scratch
DisableDirPage=yes
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
UsedUserAreasWarning=no
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=..\runner\resources\app_icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
OutputDir={#OutputDir}
OutputBaseFilename=Scratch-Setup-{#AppVersion}-{#Sha7}
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Close a running Scratch through the Restart Manager before files are replaced.
; It is not restarted by the Restart Manager; the finish page offers "Launch Scratch".
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; IconFilename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; WorkingDir: "{app}"; IconFilename: "{app}\{#AppExe}"; Tasks: desktopicon

[Registry]
; The notes:// scheme, pointing at the installed notes.exe, so page links work
; right after install (an older copy may still own the key). The app rewrites
; the same values on every start (windows/runner/main.cpp). Removed on uninstall.
Root: HKCU; Subkey: "Software\Classes\notes"; ValueType: string; ValueName: ""; ValueData: "URL:Notes page link"; Flags: uninsdeletekey
Root: HKCU; Subkey: "Software\Classes\notes"; ValueType: string; ValueName: "URL Protocol"; ValueData: ""
Root: HKCU; Subkey: "Software\Classes\notes\DefaultIcon"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"",0"
Root: HKCU; Subkey: "Software\Classes\notes\shell\open\command"; ValueType: string; ValueName: ""; ValueData: """{app}\{#AppExe}"" ""%1"""

[Run]
Filename: "{app}\{#AppExe}"; Description: "Launch {#AppName}"; Flags: nowait postinstall skipifsilent
