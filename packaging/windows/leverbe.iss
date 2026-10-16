; Inno Setup script for the LeVerbe Windows installer.
;   iscc /DAppVersion=1.2.3 /DSourceDir=<staged app dir> /DOutputDir=<dir> leverbe.iss
; <staged app dir> holds LeVerbe.exe and data\bible.db (see .github/workflows/release.yml).

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceDir
  #define SourceDir "..\..\dist\LeVerbe"
#endif
#ifndef OutputDir
  #define OutputDir "..\..\dist"
#endif

[Setup]
AppId={{6F0B8E2A-3C1D-4E7B-9A55-2D4C8B1E7F30}
AppName=LeVerbe
AppVersion={#AppVersion}
AppPublisher=LeVerbe
AppPublisherURL=https://github.com/rmpr/logos-rs
DefaultDirName={autopf}\LeVerbe
DefaultGroupName=LeVerbe
DisableProgramGroupPage=yes
UninstallDisplayIcon={app}\LeVerbe.exe
OutputDir={#OutputDir}
OutputBaseFilename=LeVerbe-{#AppVersion}-windows-x64-setup
Compression=lzma2/max
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
PrivilegesRequired=lowest
PrivilegesRequiredOverridesAllowed=dialog
WizardStyle=modern

[Languages]
Name: "english"; MessagesFile: "compiler:Default.isl"
Name: "french"; MessagesFile: "compiler:Languages\French.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "{#SourceDir}\LeVerbe.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#SourceDir}\data\bible.db"; DestDir: "{app}\data"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\LeVerbe"; Filename: "{app}\LeVerbe.exe"
Name: "{autodesktop}\LeVerbe"; Filename: "{app}\LeVerbe.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\LeVerbe.exe"; Description: "{cm:LaunchProgram,LeVerbe}"; Flags: nowait postinstall skipifsilent
