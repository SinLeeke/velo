; Mantener este AppId entre versiones para actualizar la misma instalación.
[Setup]
AppId={{F1D6AC92-628A-4B70-89EC-8E70AA1CF921}
AppName=Velo
AppVersion=0.9.1
AppPublisher=SinLeeke
AppPublisherURL=https://github.com/SinLeeke/velo
SourceDir=..
DefaultDirName={localappdata}\Programs\Velo
DefaultGroupName=Velo
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
SetupIconFile=crates\app\assets\velo.ico
UninstallDisplayIcon={app}\velo.exe
Uninstallable=yes
LicenseFile=LICENSE
Compression=lzma2/ultra64
SolidCompression=yes
OutputDir=dist
OutputBaseFilename=Velo-Setup-0.9.1

[Languages]
Name: "spanish"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"; Flags: unchecked

[Files]
Source: "dist\Velo\*"; DestDir: "{app}"; Flags: ignoreversion recursesubdirs createallsubdirs

[Icons]
Name: "{group}\Velo"; Filename: "{app}\velo.exe"; WorkingDir: "{app}"
Name: "{autodesktop}\Velo"; Filename: "{app}\velo.exe"; WorkingDir: "{app}"; Tasks: desktopicon

[Run]
Filename: "{app}\velo.exe"; Description: "{cm:LaunchProgram,Velo}"; Flags: nowait postinstall skipifsilent unchecked

; Sin UninstallDelete ni código que toque ajustes, caché o keyring:
; el desinstalador elimina únicamente archivos instalados y accesos directos.
