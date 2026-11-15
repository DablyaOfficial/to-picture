# Third-party components

To Picture's own code is MIT-licensed (see [LICENSE](LICENSE)). The installer and the portable
build also contain programs by other authors; they are distributed unmodified, under their own terms.

| Component | What it does | License | Source code |
|---|---|---|---|
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | downloads video, audio and subtitles | Unlicense | https://github.com/yt-dlp/yt-dlp |
| [FFmpeg](https://ffmpeg.org) ([yt-dlp/FFmpeg-Builds](https://github.com/yt-dlp/FFmpeg-Builds), win64-gpl-shared) | transcodes and assembles the file | GPL-3.0 (with libx264, libx265) | https://github.com/yt-dlp/FFmpeg-Builds/releases and https://ffmpeg.org/download.html#get-sources |
| [pywebview](https://github.com/r0x0r/pywebview) | the app window on WebView2 | BSD-3-Clause | https://github.com/r0x0r/pywebview |
| [pythonnet](https://github.com/pythonnet/pythonnet) | Python ↔ .NET bridge for the window | MIT | https://github.com/pythonnet/pythonnet |
| [Python](https://www.python.org) | runtime (bundled into the exe by PyInstaller) | PSF License | https://www.python.org/downloads/source/ |
| [PyInstaller](https://pyinstaller.org) | packs the app into an exe | GPL-2.0 with an exception for bundled apps | https://github.com/pyinstaller/pyinstaller |
| [Microsoft Edge WebView2](https://developer.microsoft.com/microsoft-edge/webview2/) | window engine; installed by the setup if missing | Microsoft license | — |
| [Inno Setup](https://jrsoftware.org/isinfo.php) | the installer | Inno Setup License | https://github.com/jrsoftware/issrc |

The FFmpeg license text ships next to the program: `bin/ffmpeg-LICENSE.txt`.
