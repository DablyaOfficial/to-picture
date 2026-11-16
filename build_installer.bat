@echo off
chcp 65001 >nul
rem Собирает "To Picture.exe" (Rust), установщик и портативную версию в installer\output
rem Нужны: Rust (https://rustup.rs) + Visual Studio Build Tools (C++)  +  Inno Setup 6 или 7 (https://jrsoftware.org/isdl.php)
rem yt-dlp.exe и ffmpeg (сборка win64-gpl-shared) должны лежать в installer\payload\bin
cd /d "%~dp0"

for /f "tokens=2 delims==" %%v in ('findstr /b /c:"version = " Cargo.toml') do if not defined VER set VER=%%~v
set VER=%VER: =%
set VER=%VER:"=%
echo Версия: %VER%

cargo build --release -p topicture || goto :fail
copy /y "target\release\topicture.exe" "installer\payload\To Picture.exe" >nul || goto :fail

set ISCC=
for %%p in ("%LOCALAPPDATA%\Programs\Inno Setup 7\ISCC.exe" "%LOCALAPPDATA%\Programs\Inno Setup 6\ISCC.exe" ^
            "%ProgramFiles%\Inno Setup 7\ISCC.exe" "%ProgramFiles(x86)%\Inno Setup 6\ISCC.exe" ^
            "%ProgramFiles%\Inno Setup 6\ISCC.exe") do if exist %%p if not defined ISCC set ISCC=%%p
if not defined ISCC (
  echo Inno Setup не найден. Установите его: https://jrsoftware.org/isdl.php
  goto :fail
)
%ISCC% /Q /DAppVersion=%VER% "installer\ToPicture.iss" || goto :fail

rem ---- портативная версия: папка "To Picture" с exe, bin и portable.txt, упакованная в zip
set PORT=%TEMP%\topicture_portable
if exist "%PORT%" rmdir /s /q "%PORT%"
mkdir "%PORT%\To Picture"
copy /y "installer\payload\To Picture.exe" "%PORT%\To Picture\" >nul
xcopy /e /i /q /y "installer\payload\bin" "%PORT%\To Picture\bin" >nul
copy /y "installer\portable.txt" "%PORT%\To Picture\" >nul
xcopy /e /i /q /y "extension" "%PORT%\To Picture\extension" >nul
if exist "installer\output\ToPicture-Portable.zip" del "installer\output\ToPicture-Portable.zip"
powershell -NoProfile -Command "Compress-Archive -Path '%PORT%\To Picture' -DestinationPath '%~dp0installer\output\ToPicture-Portable.zip' -CompressionLevel Optimal" || goto :fail
rmdir /s /q "%PORT%"

echo.
echo Готово:
echo   %~dp0installer\output\ToPicture-Setup.exe
echo   %~dp0installer\output\ToPicture-Portable.zip
pause
exit /b 0

:fail
echo.
echo Сборка не удалась.
pause
exit /b 1
