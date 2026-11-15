; Установщик To Picture (Inno Setup 6/7).
; Собирается через ..\build_installer.bat — он сначала собирает exe, потом этот скрипт.
;
; Ставит программу в профиль пользователя (%LOCALAPPDATA%\Programs\To Picture) без прав
; администратора — поэтому yt-dlp.exe внутри может сам себя обновлять.
; Вместе с программой кладёт свои yt-dlp.exe и ffmpeg/ffprobe (папка bin).
; Если в системе нет WebView2 (движок окна), докачивает и ставит его.

#define AppName "To Picture"
#ifndef AppVersion
  #define AppVersion "1.2.0"
#endif
#define AppExe "To Picture.exe"

[Setup]
AppId={{8C7A2E3B-5D41-4F2A-9B6E-3F1D2A7C9E01}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=output
OutputBaseFilename=ToPicture-Setup-{#AppVersion}
SetupIconFile=..\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/ultra64
SolidCompression=yes
LZMANumBlockThreads=4
WizardStyle=modern
CloseApplications=yes
RestartApplications=no

[Languages]
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"

[Tasks]
Name: "desktopicon"; Description: "Создать ярлык на рабочем столе"; GroupDescription: "Ярлыки:"

[Files]
Source: "payload\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "payload\bin\*"; DestDir: "{app}\bin"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "Запустить {#AppName}"; Flags: nowait postinstall skipifsilent

[UninstallDelete]
; yt-dlp обновляет себя сам, поэтому в bin могут появиться файлы, которых не было при установке
Type: filesandordirs; Name: "{app}\bin"

[Code]
const
  WV2Guid = '{F3017226-FE2A-4295-8BDF-00C3A9A7E4C5}';
  WV2Url = 'https://go.microsoft.com/fwlink/p/?LinkId=2124703';

var
  DownloadPage: TDownloadWizardPage;

function WV2Version(Root: Integer; Key: String): String;
begin
  if not RegQueryStringValue(Root, Key, 'pv', Result) then Result := '';
end;

function WebView2Installed: Boolean;
var
  V: String;
begin
  V := WV2Version(HKLM, 'SOFTWARE\WOW6432Node\Microsoft\EdgeUpdate\Clients\' + WV2Guid);
  if (V = '') or (V = '0.0.0.0') then
    V := WV2Version(HKLM, 'SOFTWARE\Microsoft\EdgeUpdate\Clients\' + WV2Guid);
  if (V = '') or (V = '0.0.0.0') then
    V := WV2Version(HKCU, 'Software\Microsoft\EdgeUpdate\Clients\' + WV2Guid);
  Result := (V <> '') and (V <> '0.0.0.0');
end;

procedure InitializeWizard;
begin
  DownloadPage := CreateDownloadPage(SetupMessage(msgWizardPreparing),
    'Скачивание компонента Microsoft WebView2…', nil);
end;

function NextButtonClick(CurPageID: Integer): Boolean;
var
  Code: Integer;
begin
  Result := True;
  if (CurPageID = wpReady) and not WebView2Installed then
  begin
    DownloadPage.Clear;
    DownloadPage.Add(WV2Url, 'MicrosoftEdgeWebview2Setup.exe', '');
    DownloadPage.Show;
    try
      try
        DownloadPage.Download;
        if not Exec(ExpandConstant('{tmp}\MicrosoftEdgeWebview2Setup.exe'), '/silent /install', '',
                    SW_HIDE, ewWaitUntilTerminated, Code) or not WebView2Installed then
          MsgBox('Не удалось установить Microsoft WebView2. Программа может не запуститься — ' +
                 'установите WebView2 Runtime с сайта Microsoft и запустите её снова.', mbError, MB_OK);
      except
        MsgBox('Не удалось скачать Microsoft WebView2: ' + GetExceptionMessage + #13#10 +
               'Проверьте интернет или установите WebView2 Runtime вручную.', mbError, MB_OK);
      end;
    finally
      DownloadPage.Hide;
    end;
  end;
end;
