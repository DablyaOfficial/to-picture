; To Picture installer (Inno Setup 6/7).
; Built by ..\build_installer.bat: it builds the exe first, then compiles this script.
;
; Installs per user (%LOCALAPPDATA%\Programs\To Picture) without admin rights, so the bundled
; yt-dlp.exe can update itself. Ships its own yt-dlp.exe and ffmpeg/ffprobe in the bin folder.
; The wizard language follows the Windows UI language.

#define AppName "To Picture"
#ifndef AppVersion
  #define AppVersion "1.0.1"
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
; Restart Manager не всегда закрывает программу — программу закрывает PrepareToInstall (см. [Code])
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

en.BrowserExt=Browser extension (Chrome, Edge and others)
ru.BrowserExt=Расширение для браузера (Chrome, Edge и другие)
zh.BrowserExt=浏览器扩展(Chrome、Edge 等)
ja.BrowserExt=ブラウザー拡張機能(Chrome、Edge など)
ko.BrowserExt=브라우저 확장 프로그램(Chrome, Edge 등)
es.BrowserExt=Extensión del navegador (Chrome, Edge y otros)
pt.BrowserExt=Extensão do navegador (Chrome, Edge e outros)
de.BrowserExt=Browser-Erweiterung (Chrome, Edge und andere)
fr.BrowserExt=Extension de navigateur (Chrome, Edge et autres)
tr.BrowserExt=Tarayıcı uzantısı (Chrome, Edge ve diğerleri)

[Tasks]
Name: "desktopicon"; Description: "{cm:CreateDesktopIcon}"; GroupDescription: "{cm:AdditionalIcons}"
; связь с расширением (native messaging) — записи в HKCU, без прав администратора
Name: "browserext"; Description: "{cm:BrowserExt}"

[Files]
Source: "payload\{#AppExe}"; DestDir: "{app}"; Flags: ignoreversion
Source: "payload\bin\*"; DestDir: "{app}\bin"; Excludes: "yt-dlp.exe"; Flags: ignoreversion recursesubdirs
; yt-dlp обновляет себя сам — при переустановке не откатываем его на версию из установщика
Source: "payload\bin\yt-dlp.exe"; DestDir: "{app}\bin"; Flags: onlyifdoesntexist
; папка расширения ставится всегда: подключить его можно и позже (меню ⋯ → Расширение для браузера)
Source: "..\extension\*"; DestDir: "{app}\extension"; Flags: ignoreversion recursesubdirs

[Icons]
Name: "{autoprograms}\{#AppName}"; Filename: "{app}\{#AppExe}"
Name: "{autodesktop}\{#AppName}"; Filename: "{app}\{#AppExe}"; Tasks: desktopicon

[Run]
Filename: "{app}\{#AppExe}"; Parameters: "--register-extension"; Tasks: browserext; Flags: runhidden waituntilterminated
Filename: "{app}\{#AppExe}"; Description: "{cm:LaunchProgram,{#AppName}}"; Flags: nowait postinstall skipifsilent

[UninstallRun]
Filename: "{app}\{#AppExe}"; Parameters: "--unregister-extension"; Flags: runhidden waituntilterminated; RunOnceId: "UnregisterExtension"

[UninstallDelete]
; yt-dlp обновляет себя сам, поэтому в bin могут появиться файлы, которых не было при установке
Type: filesandordirs; Name: "{app}\bin"

[Code]
// To Picture из папки установки: окно не всегда закрывается через Restart Manager, а процесс-загрузчик
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
