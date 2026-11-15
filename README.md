<p align="center">
  <img src="icon.png" width="96" alt="">
</p>

<h1 align="center">To Picture</h1>

<p align="center">
  Видео с YouTube — в файл, который открывается везде.<br>
  Лучшее качество, нужный кодек, озвучки и субтитры дорожками в одном файле.
</p>

<p align="center">
  <a href="https://github.com/DablyaOfficial/to-picture/releases/latest"><img alt="Скачать для Windows" src="https://img.shields.io/badge/Скачать-Windows_10%2F11-B4501E?style=for-the-badge"></a>
  <a href="https://github.com/DablyaOfficial/to-picture/releases/latest"><img alt="Последняя версия" src="https://img.shields.io/github/v/release/DablyaOfficial/to-picture?style=for-the-badge&label=версия&color=5C5C58"></a>
  <a href="LICENSE"><img alt="Лицензия MIT" src="https://img.shields.io/badge/лицензия-MIT-5C5C58?style=for-the-badge"></a>
</p>

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/screenshot-dark.png">
    <img src="docs/screenshot-light.png" width="720" alt="Окно To Picture: список ссылок, превью видео с озвучками и субтитрами, параметры">
  </picture>
</p>

## Что умеет

- **Вставил ссылку — видишь, что скачаешь.** Обложка, название, канал, длительность, доступное качество, озвучки и субтитры — до скачивания.
- **Лучшее качество, нужный формат.** H.264 (открывается везде), H.265, ProRes 422 для монтажа или исходник без перекодирования.
- **1080p — за секунды.** Если YouTube уже отдаёт H.264 нужного разрешения, видео не перекодируется, а только перепаковывается: быстрее и без потери качества.
- **Кодирует на видеокарте.** NVIDIA, AMD или Intel находятся сами — в разы быстрее и почти без нагрузки на память. Нет видеокарты — работает процессор.
- **Озвучки и субтитры — дорожками в одном файле.** Оригинал + русская (или любые другие) и официальные субтитры встраиваются в файл; переключаются прямо в плеере. Автоматические субтитры YouTube не скачиваются.
- **Очередь.** Несколько ссылок — по одной на строку, с прогрессом и оставшимся временем.
- **Ничего не надо настраивать.** yt-dlp и ffmpeg уже внутри, yt-dlp обновляется сам. Светлая и тёмная тема.

## Установка

Скачай со страницы [**Releases**](https://github.com/DablyaOfficial/to-picture/releases/latest) один из файлов:

| Файл | Для кого |
|---|---|
| `ToPicture-Setup-<версия>.exe` | Обычная установка. Права администратора не нужны, ярлык в «Пуске», обновляется поверх старой версии, удаляется через «Параметры → Приложения». |
| `ToPicture-Portable-<версия>.zip` | Без установки: распаковать и запустить `To Picture.exe`. Настройки хранятся рядом, в папке `data`. Подходит для флешки. |

Нужен Windows 10 или 11 (64-бит). Движок окна Microsoft WebView2 в Windows 11 уже есть; на Windows 10 установщик докачает его сам.

> [!NOTE]
> Установщик не подписан цифровой подписью, поэтому при первом запуске Windows SmartScreen может показать «Windows защитила ваш компьютер». Нажми **Подробнее → Выполнить в любом случае**.

## Как пользоваться

1. Скопируй ссылку на видео и нажми **Вставить** (или Ctrl+V в окне). Можно несколько ссылок — по одной на строку.
2. Посмотри превью: качество, во что перекодируется, какие озвучки и субтитры попадут в файл. Если у видео много озвучек, нужные можно отметить прямо в превью — только для этой ссылки.
3. Нажми **Скачать** (или Ctrl+Enter). Готовый файл — кнопка **Показать в папке**.

Настройки по умолчанию подходят большинству: H.264, лучшее качество, оригинал + русская озвучка, русские и английские субтитры. Всё меняется в разделе **Параметры**; тема — в меню **⋯**.

<p align="center">
  <picture>
    <source media="(prefers-color-scheme: dark)" srcset="docs/screenshot-dark.png">
    <img src="docs/screenshot-dark.png" width="720" alt="To Picture во время работы: скачивание завершено, идёт перекодирование на видеокарте">
  </picture>
</p>

## Частые вопросы

<details>
<summary><b>Почему файл .mkv, а не .mp4?</b></summary>

Когда в видео встроены субтитры, программа сохраняет его в MKV: в MP4 первые субтитры всегда помечаются как «включены», и плееры показывают их сразу. MKV открывают VLC, MPC, PotPlayer и «Медиаплеер» Windows 11. Если нужен MP4 — выключи субтитры в **Параметрах** или выбери «Встроить и сохранить .srt» и используй отдельный файл.
</details>

<details>
<summary><b>Видео не скачивается / ошибка 403</b></summary>

Чаще всего YouTube что-то поменял, и нужна свежая версия yt-dlp. Программа обновляет его сама раз в сутки и сразу, если загрузка падает с такой ошибкой, — попробуй ещё раз через минуту. Подробности — в разделе **Журнал** внизу окна.
</details>

<details>
<summary><b>Перекодирование в 4K долгое</b></summary>

YouTube отдаёт 4K только в VP9/AV1, поэтому для H.264 его приходится перекодировать. На видеокарте это примерно в 1,5 раза быстрее реального времени для 4K60. Если 4K не нужен — выбери 1080p в **Разрешении**: тогда видео обычно вообще не перекодируется.
</details>

<details>
<summary><b>Где хранятся настройки?</b></summary>

В `%APPDATA%\ToPicture` (портативная версия — в папке `data` рядом с программой). Открыть папку: меню **⋯ → Папка настроек**.
</details>

## Сборка из исходников

Нужны Python 3.12+, [Inno Setup 6/7](https://jrsoftware.org/isdl.php) (для установщика), а также `yt-dlp.exe` и ffmpeg ([сборка yt-dlp](https://github.com/yt-dlp/FFmpeg-Builds/releases), `win64-gpl-shared`) в папке `installer/payload/bin`.

```bat
pip install -r requirements.txt
python ToPicture.pyw            :: запуск из исходников (yt-dlp и ffmpeg — в PATH или в bin рядом)
build_installer.bat             :: exe + установщик + портативный zip в installer\output
```

Устройство: интерфейс — [`ui/index.html`](ui/index.html) (HTML/CSS в окне [pywebview](https://github.com/r0x0r/pywebview) на WebView2), логика — [`ToPicture.pyw`](ToPicture.pyw), установщик — [`installer/ToPicture.iss`](installer/ToPicture.iss).

## Лицензии

Код To Picture — [MIT](LICENSE). Установщик и портативная версия содержат [yt-dlp](https://github.com/yt-dlp/yt-dlp) (Unlicense) и [FFmpeg](https://ffmpeg.org) (GPL-3.0) без изменений — подробности и ссылки на исходники в [THIRD-PARTY-NOTICES.md](THIRD-PARTY-NOTICES.md).

Скачивай только то, на что у тебя есть права, и соблюдай условия YouTube.

<sub>На скриншотах — открытые фильмы Blender Studio «Sprite Fright» и «CHARGE» (© Blender Foundation, CC BY 4.0).</sub>
