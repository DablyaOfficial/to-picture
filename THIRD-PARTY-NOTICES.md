# Third-party components

To Picture's own code is MIT-licensed (see [LICENSE](LICENSE)). The installer and the portable
build also contain programs by other authors; they are distributed unmodified, under their own terms.

| Component | What it does | License | Source code |
|---|---|---|---|
| [yt-dlp](https://github.com/yt-dlp/yt-dlp) | downloads video, audio and subtitles | Unlicense | https://github.com/yt-dlp/yt-dlp |
| [FFmpeg](https://ffmpeg.org) ([yt-dlp/FFmpeg-Builds](https://github.com/yt-dlp/FFmpeg-Builds), win64-gpl-shared) | transcodes and assembles the file | GPL-3.0 (with libx264, libx265) | https://github.com/yt-dlp/FFmpeg-Builds/releases and https://ffmpeg.org/download.html#get-sources |
| [Slint](https://slint.dev) | the app window (native UI toolkit) | Slint Royalty-free Desktop License 2.0 | https://github.com/slint-ui/slint |
| [Skia](https://skia.org) (via Slint) | draws the window | BSD-3-Clause | https://skia.googlesource.com/skia |
| Rust crates: serde, serde_json, regex, ureq, image, arboard, rfd, winreg, windows-sys, rand, tempfile, skrifa | JSON, HTTP, images, clipboard, dialogs, Windows APIs | MIT / Apache-2.0 | https://crates.io |
| [Inno Setup](https://jrsoftware.org/isinfo.php) | the installer | Inno Setup License | https://github.com/jrsoftware/issrc |

The FFmpeg license text ships next to the program: `bin/ffmpeg-LICENSE.txt`.

The window is built with Slint: [![Made with Slint](https://raw.githubusercontent.com/slint-ui/slint/master/logo/MadeWithSlint-logo-whitebg.png)](https://slint.dev)
