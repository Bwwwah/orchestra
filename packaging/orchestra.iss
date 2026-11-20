; Windows installer (Inno Setup), built by the release workflow:
;   ISCC /DAppVersion=1.2.3 /DSourceExe=...\orchestra.exe /DOutDir=...\out packaging\orchestra.iss
; Installs for the current user (no admin rights), so the app can update itself.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#ifndef SourceExe
  #define SourceExe "..\target\release\orchestra.exe"
#endif
#ifndef OutDir
  #define OutDir "..\dist"
#endif

[Setup]
AppId={{8F3C2A51-6B2E-4C8E-9C5B-0A7E1D2F4B11}
AppName=Orchestra
AppVersion={#AppVersion}
AppPublisher=Orchestra
DefaultDirName={localappdata}\Programs\Orchestra
DefaultGroupName=Orchestra
DisableProgramGroupPage=yes
PrivilegesRequired=lowest
OutputDir={#OutDir}
OutputBaseFilename=Orchestra-v{#AppVersion}-Windows-Setup
SetupIconFile=..\assets\icon.ico
UninstallDisplayIcon={app}\orchestra.exe
Compression=lzma2
SolidCompression=yes
WizardStyle=modern
; Orchestra still running (the window, its terminals, the MCP servers its AIs started)
; holds orchestra.exe: closed first, see [Code].
CloseApplications=force
RestartApplications=no

[Languages]
Name: "french"; MessagesFile: "compiler:Languages\French.isl"
Name: "english"; MessagesFile: "compiler:Default.isl"

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "{#SourceExe}"; DestDir: "{app}"; DestName: "orchestra.exe"; Flags: ignoreversion

[Icons]
Name: "{autoprograms}\Orchestra"; Filename: "{app}\orchestra.exe"
Name: "{autodesktop}\Orchestra"; Filename: "{app}\orchestra.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\orchestra.exe"; Description: "{cm:LaunchProgram,Orchestra}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
; Before the files go: Claude Code's status line put back, Orchestra unplugged from the AIs,
; so nothing keeps trying to start the removed orchestra.exe.
Filename: "{app}\orchestra.exe"; Parameters: "nettoyer"; Flags: runhidden waituntilterminated; RunOnceId: "OrchestraNettoyer"

[Code]
// Every orchestra.exe (the window, `orchestra mcp` started by the AIs, `orchestra
// statusline`) and everything they started (PowerShell, claude, node…): /T ends the whole
// tree, /F without waiting. Otherwise orchestra.exe stays locked and the install over an
// old version, or the uninstall, fails on it.
procedure StopOrchestra();
var
  Code: Integer;
begin
  Exec(ExpandConstant('{sys}\taskkill.exe'), '/F /T /IM orchestra.exe', '', SW_HIDE, ewWaitUntilTerminated, Code);
  // Let Windows release the file handles.
  Sleep(800);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  StopOrchestra();
  Result := '';
end;

function InitializeUninstall(): Boolean;
begin
  StopOrchestra();
  Result := True;
end;
