; To Picture installer (Inno Setup 6/7).
; Built by ..\build_installer.bat: it builds the exe first, then compiles this script.
;
; Installs per user (%LOCALAPPDATA%\Programs\To Picture) without admin rights, so the bundled
; yt-dlp.exe can update itself. Ships its own yt-dlp.exe and ffmpeg/ffprobe in the bin folder.
; Downloads and installs Microsoft WebView2 (the window engine) if it is missing.
; The wizard language follows the Windows UI language.

#define AppName "To Picture"
#ifndef AppVersion
  #define AppVersion "1.7.0"
#endif
#define AppExe "To Picture.exe"

[Setup]
AppId={{8C7A2E3B-5D41-4F2A-9B6E-3F1D2A7C9E01}
AppName={#AppName}
AppVersion={#AppVersion}
AppVerName={#AppName} {#AppVersion}
AppPublisher=DablyaOfficial
AppPublisherURL=https://github.com/DablyaOfficial/to-picture
AppSupportURL=https://github.com/DablyaOfficial/to-picture/issues
AppUpdatesURL=https://github.com/DablyaOfficial/to-picture/releases
VersionInfoVersion={#AppVersion}
DefaultDirName={autopf}\{#AppName}
DefaultGroupName={#AppName}
DisableProgramGroupPage=yes
DisableDirPage=auto
PrivilegesRequired=lowest
ArchitecturesAllowed=x64compatible
ArchitecturesInstallIn64BitMode=x64compatible
MinVersion=10.0
OutputDir=output
; имя без версии — тогда ссылка .../releases/latest/download/ToPicture-Setup.exe всегда ведёт на последнюю версию
OutputBaseFilename=ToPicture-Setup
SetupIconFile=..\icon.ico
UninstallDisplayIcon={app}\{#AppExe}
UninstallDisplayName={#AppName}
Compression=lzma2/ultra64
SolidCompression=yes
LZMANumBlockThreads=4
WizardStyle=modern
; Restart Manager не умеет закрывать окно pywebview — программу закрывает PrepareToInstall (см. [Code])
CloseApplications=no
RestartApplications=no
ShowLanguageDialog=auto

[Languages]
Name: "en"; MessagesFile: "compiler:Default.isl"
Name: "ru"; MessagesFile: "compiler:Languages\Russian.isl"
Name: "zh"; MessagesFile: "compiler:Languages\ChineseSimplified.isl"
Name: "ja"; MessagesFile: "compiler:Languages\Japanese.isl"
Name: "ko"; MessagesFile: "compiler:Languages\Korean.isl"
Name: "es"; MessagesFile: "compiler:Languages\Spanish.isl"
Name: "pt"; MessagesFile: "compiler:Languages\BrazilianPortuguese.isl"
Name: "de"; MessagesFile: "compiler:Languages\German.isl"
Name: "fr"; MessagesFile: "compiler:Languages\French.isl"
Name: "tr"; MessagesFile: "compiler:Languages\Turkish.isl"

[CustomMessages]
en.WV2Downloading=Downloading Microsoft WebView2…
en.WV2InstallFailed=Microsoft WebView2 could not be installed. The app may not start — install WebView2 Runtime from Microsoft and run it again.
en.WV2DownloadFailed=Microsoft WebView2 could not be downloaded:
en.WV2CheckNet=Check your internet connection or install WebView2 Runtime manually.
ru.WV2Downloading=Скачивание компонента Microsoft WebView2…
ru.WV2InstallFailed=Не удалось установить Microsoft WebView2. Программа может не запуститься — установите WebView2 Runtime с сайта Microsoft и запустите её снова.
ru.WV2DownloadFailed=Не удалось скачать Microsoft WebView2:
ru.WV2CheckNet=Проверьте интернет или установите WebView2 Runtime вручную.
zh.WV2Downloading=正在下载 Microsoft WebView2…
zh.WV2InstallFailed=无法安装 Microsoft WebView2，程序可能无法启动。请从 Microsoft 网站安装 WebView2 Runtime 后重新运行。
zh.WV2DownloadFailed=无法下载 Microsoft WebView2：
zh.WV2CheckNet=请检查网络，或手动安装 WebView2 Runtime。
ja.WV2Downloading=Microsoft WebView2 をダウンロード中…
ja.WV2InstallFailed=Microsoft WebView2 をインストールできませんでした。アプリが起動しない場合は、Microsoft のサイトから WebView2 Runtime をインストールしてからもう一度実行してください。
ja.WV2DownloadFailed=Microsoft WebView2 をダウンロードできませんでした：
ja.WV2CheckNet=インターネット接続を確認するか、WebView2 Runtime を手動でインストールしてください。
ko.WV2Downloading=Microsoft WebView2 다운로드 중…
ko.WV2InstallFailed=Microsoft WebView2를 설치하지 못했습니다. 프로그램이 실행되지 않을 수 있습니다. Microsoft 웹사이트에서 WebView2 Runtime을 설치한 뒤 다시 실행하세요.
ko.WV2DownloadFailed=Microsoft WebView2를 다운로드하지 못했습니다:
ko.WV2CheckNet=인터넷 연결을 확인하거나 WebView2 Runtime을 직접 설치하세요.
es.WV2Downloading=Descargando Microsoft WebView2…
es.WV2InstallFailed=No se pudo instalar Microsoft WebView2. Puede que la aplicación no se inicie: instala WebView2 Runtime desde el sitio de Microsoft y vuelve a ejecutarla.
es.WV2DownloadFailed=No se pudo descargar Microsoft WebView2:
es.WV2CheckNet=Comprueba la conexión a internet o instala WebView2 Runtime manualmente.
pt.WV2Downloading=Baixando o Microsoft WebView2…
pt.WV2InstallFailed=Não foi possível instalar o Microsoft WebView2. O aplicativo pode não abrir — instale o WebView2 Runtime pelo site da Microsoft e execute-o de novo.
pt.WV2DownloadFailed=Não foi possível baixar o Microsoft WebView2:
pt.WV2CheckNet=Verifique a internet ou instale o WebView2 Runtime manualmente.
de.WV2Downloading=Microsoft WebView2 wird heruntergeladen…
de.WV2InstallFailed=Microsoft WebView2 konnte nicht installiert werden. Die App startet eventuell nicht — installieren Sie WebView2 Runtime von Microsoft und starten Sie sie erneut.
de.WV2DownloadFailed=Microsoft WebView2 konnte nicht heruntergeladen werden:
de.WV2CheckNet=Prüfen Sie die Internetverbindung oder installieren Sie WebView2 Runtime manuell.
fr.WV2Downloading=Téléchargement de Microsoft WebView2…
fr.WV2InstallFailed=Impossible d’installer Microsoft WebView2. L’application risque de ne pas démarrer — installez WebView2 Runtime depuis le site de Microsoft puis relancez-la.
fr.WV2DownloadFailed=Impossible de télécharger Microsoft WebView2 :
fr.WV2CheckNet=Vérifiez la connexion Internet ou installez WebView2 Runtime manuellement.
tr.WV2Downloading=Microsoft WebView2 indiriliyor…
tr.WV2InstallFailed=Microsoft WebView2 yüklenemedi. Uygulama açılmayabilir — Microsoft sitesinden WebView2 Runtime yükleyip yeniden çalıştırın.
tr.WV2DownloadFailed=Microsoft WebView2 indirilemedi:
tr.WV2CheckNet=İnternet bağlantısını kontrol edin veya WebView2 Runtime’ı elle yükleyin.

en.AppRunning=To Picture is running. Close it and continue the installation?
en.AppRunningAbort=Close To Picture and run the installer again.
ru.AppRunning=Программа To Picture запущена. Закрыть её и продолжить установку?
ru.AppRunningAbort=Закройте To Picture и запустите установку снова.
zh.AppRunning=To Picture 正在运行。要关闭它并继续安装吗？
zh.AppRunningAbort=请关闭 To Picture 后重新运行安装程序。
ja.AppRunning=To Picture が実行中です。終了してインストールを続けますか？
ja.AppRunningAbort=To Picture を終了してから、もう一度インストーラーを実行してください。
ko.AppRunning=To Picture가 실행 중입니다. 종료하고 설치를 계속할까요?
ko.AppRunningAbort=To Picture를 종료한 뒤 설치 프로그램을 다시 실행하세요.
es.AppRunning=To Picture está abierto. ¿Cerrarlo y continuar con la instalación?
es.AppRunningAbort=Cierra To Picture y vuelve a ejecutar el instalador.
pt.AppRunning=O To Picture está aberto. Fechá-lo e continuar a instalação?
pt.AppRunningAbort=Feche o To Picture e execute o instalador de novo.
de.AppRunning=To Picture läuft gerade. Schließen und mit der Installation fortfahren?
de.AppRunningAbort=Schließen Sie To Picture und starten Sie die Installation erneut.
fr.AppRunning=To Picture est ouvert. Le fermer et poursuivre l’installation ?
fr.AppRunningAbort=Fermez To Picture et relancez l’installation.
tr.AppRunning=To Picture çalışıyor. Kapatıp kuruluma devam edilsin mi?
tr.AppRunningAbort=To Picture’ı kapatıp kurulumu yeniden başlatın.

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"

[Files]
Source: "payload\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "payload\bin\*"; DestDir: "{app}\bin"; Excludes: "yt-dlp.exe"; Flags: ignoreversion recursesubdirs
; yt-dlp обновляет себя сам — при переустановке не откатываем его на версию из установщика
Source: "payload\bin\yt-dlp.exe"; DestDir: "{app}\bin"; Flags: onlyifdoesntexist

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

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

// To Picture из папки установки: окно pywebview не закрывается через Restart Manager, а процесс-загрузчик
// без окна может остаться после зависания — тогда exe нельзя заменить («Отказано в доступе»).
// Поэтому перед установкой и удалением спрашиваем и закрываем программу сами.
function RunPS(Cmd: String): Integer;
begin
  Log('powershell: ' + Cmd);
  if not Exec(ExpandConstant('{sys}\WindowsPowerShell\v1.0\powershell.exe'),
              '-NoProfile -ExecutionPolicy Bypass -Command "' + Cmd + '"', '', SW_HIDE, ewWaitUntilTerminated, Result) then
    Result := 0;
end;

// Win32_Process видит путь и у 64-битных процессов (32-битный установщик запускает 32-битный PowerShell,
// у которого Get-Process не отдаёт Path чужой разрядности)
function AppProcs(Dir: String): String;
begin
  Result := '@(Get-CimInstance Win32_Process | Where-Object { $_.Name -eq ''To Picture.exe'' -and $_.ExecutablePath -like ''' + Dir + '\*'' })';
end;

function CountRunning(Dir: String): Integer;
begin
  Result := RunPS('exit ' + AppProcs(Dir) + '.Count');
end;

procedure KillRunning(Dir: String);
begin
  RunPS(AppProcs(Dir) + ' | ForEach-Object { Stop-Process -Id $_.ProcessId -Force -ErrorAction SilentlyContinue }');
  Sleep(700);
end;

function PrepareToInstall(var NeedsRestart: Boolean): String;
var
  Dir: String;
  N: Integer;
begin
  Result := '';
  Dir := ExpandConstant('{app}');
  N := CountRunning(Dir);
  Log('To Picture processes running from ' + Dir + ': ' + IntToStr(N));
  if N > 0 then
  begin
    if SuppressibleMsgBox(CustomMessage('AppRunning'), mbConfirmation, MB_OKCANCEL, IDOK) = IDOK then
      KillRunning(Dir)
    else
      Result := CustomMessage('AppRunningAbort');
  end;
end;

function InitializeUninstall(): Boolean;
begin
  KillRunning(ExpandConstant('{app}'));
  Result := True;
end;

procedure InitializeWizard;
begin
  DownloadPage := CreateDownloadPage(SetupMessage(msgWizardPreparing), CustomMessage('WV2Downloading'), nil);
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
          MsgBox(CustomMessage('WV2InstallFailed'), mbError, MB_OK);
      except
        MsgBox(CustomMessage('WV2DownloadFailed') + ' ' + GetExceptionMessage + #13#10 +
               CustomMessage('WV2CheckNet'), mbError, MB_OK);
      end;
    finally
      DownloadPage.Hide;
    end;
  end;
end;
