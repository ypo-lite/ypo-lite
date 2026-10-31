; Ypo-Lite installer. Built by scripts\release.ps1, which passes
; /DAppVersion=<workspace version> and signs the executables beforehand.
;
; Per-user only (no elevation): the apps register themselves in HKCU\...\Run,
; and the default configuration belongs in the installing user's profile, so a
; machine-wide install would gain nothing.

#ifndef AppVersion
  #define AppVersion "0.0.0"
#endif
#define Target "..\target\release"

; The settings apps need this runtime, and windows-reactor 0.100 asks for at
; least 2.4. Newer releases are at https://aka.ms/windowsappsdk; updating this
; means updating the checksum too. Raise RuntimeMinimum with windows-reactor.
#define RuntimeMinimum "2.4.0.0"
#define RuntimeFile "windowsappruntimeinstall-x64.exe"
#define RuntimeUrl "https://aka.ms/windowsappsdk/2.6/2.6.4/" + RuntimeFile
#define RuntimeSha256 "9bf9c28bfa745a5d3c88b3da29b5c7bec0dc88b42fafa3f918ef0b85ea9bba33"

[Setup]
AppId={{B3C1F0E2-6D4A-4E0B-9A57-0C4E7A1D2F68}
AppName=Ypo-Lite
AppVersion={#AppVersion}
AppPublisher=Ypo-Lite
DefaultDirName={autopf}\Ypo-Lite
DefaultGroupName=Ypo-Lite
PrivilegesRequired=lowest
OutputDir=..\dist
OutputBaseFilename=ypo-lite-setup-{#AppVersion}
Compression=lzma2/ultra
SolidCompression=yes
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
WizardStyle=modern
UninstallDisplayName=Ypo-Lite
; Tray apps keep their exe locked: close them before replacing files.
CloseApplications=force
RestartApplications=no
; Signs the setup and uninstaller when release.ps1 defines the `signtool` command.
#ifdef Sign
SignTool=signtool
SignedUninstaller=yes
#endif

[Components]
Name: "snap"; Description: "Y-Snap (window management)"; Types: full custom
Name: "snapconfig"; Description: "Y-Snap settings"; Types: full custom
Name: "pop"; Description: "Y-Pop"; Types: full custom
Name: "popconfig"; Description: "Y-Pop settings"; Types: full custom

[Tasks]
; Y-Pop has its own "Start with Windows" menu item; Y-Snap has none.
Name: "snapstartup"; Description: "Start Y-Snap with Windows"; Components: snap; Flags: unchecked

[Files]
Source: "{#Target}\y-snap.exe"; DestDir: "{app}"; Components: snap; Flags: ignoreversion
Source: "{#Target}\y-snap-config.exe"; DestDir: "{app}"; Components: snapconfig; Flags: ignoreversion
Source: "{#Target}\y-pop.exe"; DestDir: "{app}"; Components: pop; Flags: ignoreversion
Source: "{#Target}\y-pop-config.exe"; DestDir: "{app}"; Components: popconfig; Flags: ignoreversion
Source: "..\LICENSE"; DestDir: "{app}"; Flags: ignoreversion
; Y-Snap does nothing without a configuration, and Y-Pop has an empty menu.
; An existing file is the user's own: never replace it, never remove it.
Source: "..\y-snap\rsc\y-snap.xml"; DestDir: "{userappdata}\Ypo-Lite"; Components: snap; Flags: onlyifdoesntexist uninsneveruninstall
Source: "..\y-pop\rsc\y-pop.xml"; DestDir: "{userappdata}\Ypo-Lite"; Components: pop; Flags: onlyifdoesntexist uninsneveruninstall

[Icons]
Name: "{group}\Y-Snap"; Filename: "{app}\y-snap.exe"; Components: snap
Name: "{group}\Y-Snap settings"; Filename: "{app}\y-snap-config.exe"; Components: snapconfig
Name: "{group}\Y-Pop"; Filename: "{app}\y-pop.exe"; Components: pop
Name: "{group}\Y-Pop settings"; Filename: "{app}\y-pop-config.exe"; Components: popconfig

[Run]
Filename: "{app}\y-snap.exe"; Description: "Start Y-Snap"; Components: snap; Flags: nowait postinstall skipifsilent
Filename: "{app}\y-pop.exe"; Description: "Start Y-Pop"; Components: pop; Flags: nowait postinstall skipifsilent unchecked

[UninstallRun]
; Stop the tray apps so their files can be deleted.
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM y-snap.exe"; Flags: runhidden; RunOnceId: "KillSnap"
Filename: "{sys}\taskkill.exe"; Parameters: "/F /IM y-pop.exe"; Flags: runhidden; RunOnceId: "KillPop"

[Registry]
; Written by the "Start Y-Snap with Windows" task, in the form y_core::startup uses.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueType: string; ValueName: "Y-Snap"; ValueData: """{app}\y-snap.exe"""; Tasks: snapstartup; Flags: uninsdeletevalue
; Y-Pop's own "Start with Windows" writes this value (ipc::STARTUP_NAME); remove
; it on uninstall so the Run key does not keep pointing at a deleted file.
Root: HKCU; Subkey: "Software\Microsoft\Windows\CurrentVersion\Run"; ValueName: "Y-Pop"; Flags: dontcreatekey uninsdeletevalue

; Settings in %APPDATA%\Ypo-Lite are deliberately left behind on uninstall.

[Code]
var
  DownloadPage: TDownloadWizardPage;
  RuntimeDownloaded: Boolean;

function OnDownloadProgress(const Url, FileName: String; const Progress, ProgressMax: Int64): Boolean;
begin
  Result := True;
end;

procedure InitializeWizard;
begin
  DownloadPage := CreateDownloadPage('Windows App Runtime',
    'Downloading the runtime the settings apps need...', @OnDownloadProgress);
end;

// Asks the package manager, the same source the settings apps resolve the
// runtime from. If PowerShell cannot answer, the runtime counts as missing:
// running its installer again does no harm.
function RuntimeInstalled: Boolean;
var
  ResultCode: Integer;
begin
  Result := Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
    '-NoProfile -NonInteractive -Command "if (Get-AppxPackage -Name Microsoft.WindowsAppRuntime.2 | ' +
    'Where-Object { $_.Architecture -eq ''X64'' -and [version]$_.Version -ge [version]''{#RuntimeMinimum}'' }) ' +
    '{ exit 0 } else { exit 1 }"',
    '', SW_HIDE, ewWaitUntilTerminated, ResultCode) and (ResultCode = 0);
end;

function RuntimeNeeded: Boolean;
begin
  Result := (WizardIsComponentSelected('snapconfig') or WizardIsComponentSelected('popconfig'))
    and not RuntimeInstalled;
end;

// A failed or cancelled download does not stop the install: Y-Snap and Y-Pop
// do not need the runtime, and the settings apps explain what is missing.
// A silent install never downloads 125 MB unasked.
function NextButtonClick(CurPageID: Integer): Boolean;
begin
  Result := True;
  if (CurPageID <> wpReady) or WizardSilent or not RuntimeNeeded then
    exit;

  DownloadPage.Clear;
  DownloadPage.Add('{#RuntimeUrl}', '{#RuntimeFile}', '{#RuntimeSha256}');
  DownloadPage.Show;
  try
    try
      DownloadPage.Download;
      RuntimeDownloaded := True;
    except
      if not DownloadPage.AbortedByUser then
        SuppressibleMsgBox('The Windows App Runtime could not be downloaded. ' +
          'The settings apps will offer to get it when they start.' + #13#10#13#10 +
          GetExceptionMessage, mbError, MB_OK, IDOK);
    end;
  finally
    DownloadPage.Hide;
  end;
end;

procedure CurStepChanged(CurStep: TSetupStep);
var
  ResultCode: Integer;
begin
  if (CurStep <> ssPostInstall) or not RuntimeDownloaded then
    exit;

  WizardForm.StatusLabel.Caption := 'Installing the Windows App Runtime...';
  if not Exec(ExpandConstant('{tmp}\{#RuntimeFile}'), '--quiet', '', SW_HIDE,
    ewWaitUntilTerminated, ResultCode) or (ResultCode <> 0) then
    SuppressibleMsgBox(Format('The Windows App Runtime could not be installed (code 0x%x). ' +
      'The settings apps will offer to get it when they start.', [ResultCode]),
      mbError, MB_OK, IDOK);
end;
