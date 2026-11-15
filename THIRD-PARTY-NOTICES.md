# Сторонние компоненты

Код To Picture распространяется под лицензией MIT (см. [LICENSE](LICENSE)).
Установщик и портативная версия дополнительно содержат программы других авторов — они
распространяются на своих условиях и в исходном виде, без изменений.

| Компонент | Что делает | Лицензия | Исходный код |
|---|---|---|---|
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | скачивает видео, звук и субтитры | Unlicense | https://github.com/yt-dlp/yt-dlp |
| [FFmpeg](https://ffmpeg.org) (сборка [yt-dlp/FFmpeg-Builds](https://github.com/yt-dlp/FFmpeg-Builds), win64-gpl-shared) | перекодирует и собирает файл | GPL-3.0 (с libx264, libx265) | https://github.com/yt-dlp/FFmpeg-Builds/releases и https://ffmpeg.org/download.html#get-sources |
| [pywebview](https://github.com/r0x0r/pywebview) | окно программы на WebView2 | BSD-3-Clause | https://github.com/r0x0r/pywebview |
| [pythonnet](https://github.com/pythonnet/pythonnet) | связь Python и .NET для окна | MIT | https://github.com/pythonnet/pythonnet |
| [Python](https://www.python.org) | среда выполнения (встроена в exe через PyInstaller) | PSF License | https://www.python.org/downloads/source/ |
| [PyInstaller](https://pyinstaller.org) | упаковка в exe | GPL-2.0 с исключением для собранных программ | https://github.com/pyinstaller/pyinstaller |
| [Microsoft Edge WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) | движок окна; ставится установщиком, если его нет | лицензия Microsoft | — |

Текст лицензии FFmpeg лежит рядом с программой: `bin/ffmpeg-LICENSE.txt`.
