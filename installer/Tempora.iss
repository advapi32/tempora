#ifndef AppVersion
  #error Pass the Cargo package version with /DAppVersion=...
#endif
#ifndef Sign
  #define Sign "0"
#endif

#define AppName "Tempora"
#define RunKey "Software\Microsoft\Windows\CurrentVersion\Run"
#define BuildDir "..\target\x86_64-pc-windows-msvc\release"

[Setup]
AppId={{B10129BD-624D-44F4-8570-581A6D7AA25C}
AppName={#AppName}
AppVersion={#AppVersion}
PrivilegesRequired=lowest
DefaultDirName={localappdata}\Programs\{#AppName}
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0.17763
WizardStyle=modern
SetupIconFile=..\assets\icons\app.ico
; Installed apps shows the version in its own column, so the name is just the name.
UninstallDisplayName={#AppName}
UninstallDisplayIcon={app}\tempora.exe
OutputDir=..\dist
OutputBaseFilename={#AppName}-Setup-{#AppVersion}-x64
DisableProgramGroupPage=yes
CloseApplications=yes
#if Sign == "1"
SignTool=TemporaSigning
#endif

[Files]
Source: "{#BuildDir}\tempora-recorder.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "{#BuildDir}\tempora.exe"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\README.md"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\crates\tempora-dashboard\src\dashboard\fonts\OFL-Geist.txt"; DestDir: "{app}"; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion skipifsourcedoesntexist

[Tasks]
Name: "desktopicon"; Description: "Create a desktop shortcut"; Flags: unchecked

[Icons]
Name: "{userprograms}\{#AppName}"; Filename: "{app}\tempora.exe"
Name: "{userdesktop}\{#AppName}"; Filename: "{app}\tempora.exe"; Tasks: desktopicon

[Run]
Filename: "{app}\tempora.exe"; Description: "Launch Tempora"; Flags: postinstall nowait skipifsilent

[Registry]
Root: HKCU; Subkey: "{#RunKey}"; ValueType: string; ValueName: "{#AppName}"; ValueData: """{app}\tempora-recorder.exe"" --sign-in"; Check: RunEntryExists

[Code]
function RunEntryExists: Boolean;
begin
  Result := RegValueExists(HKCU, '{#RunKey}', '{#AppName}');
end;

function StopRecorder: String;
var
  RecorderPath: String;
  ExitCode: Integer;
begin
  Result := '';
  RecorderPath := ExpandConstant('{app}\tempora-recorder.exe');
  if not FileExists(RecorderPath) then
    Exit;

  if not Exec(RecorderPath, '--quit', ExpandConstant('{app}'), SW_HIDE,
    ewWaitUntilTerminated, ExitCode) then
    Result := 'Could not run the recorder shutdown command: ' + SysErrorMessage(ExitCode)
  else if ExitCode <> 0 then
    Result := 'Could not stop the recorder cleanly (exit code ' + IntToStr(ExitCode) +
      '). Quit it from the notification area and try again.';
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
begin
  Result := StopRecorder;
end;

procedure CurUninstallStepChanged(CurUninstallStep: TUninstallStep);
var
  Error: String;
  DataDir: String;
begin
  if CurUninstallStep <> usUninstall then
    Exit;

  Error := StopRecorder;
  if Error = '' then
    if RunEntryExists and not RegDeleteValue(HKCU, '{#RunKey}', '{#AppName}') then
      Error := 'Could not remove the start-at-sign-in entry.';
  if Error <> '' then begin
    Log(Error);
    if not UninstallSilent then
      MsgBox(Error, mbError, MB_OK);
    Abort;
  end;

  DataDir := ExpandConstant('{localappdata}\Tempora');
  if not UninstallSilent and DirExists(DataDir) then
    if MsgBox('Also delete your recorded history?', mbConfirmation,
      MB_YESNO or MB_DEFBUTTON2) = IDYES then
      if not DelTree(DataDir, True, True, True) then
        MsgBox('Some history files could not be deleted from ' + DataDir,
          mbError, MB_OK);
end;
