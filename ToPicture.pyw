"""
To Picture — качает видео с YouTube в лучшем качестве и перегоняет в удобный
кодек (по умолчанию H.264 + AAC / mp4 — открывается в любом плеере и редакторе).

Интерфейс — ui/index.html (pywebview, WebView2). Логика — здесь.
yt-dlp и ffmpeg берутся из папки bin рядом с программой (её кладёт установщик),
иначе — из PATH. Собственный yt-dlp.exe обновляется сам раз в сутки.
"""

import ctypes
import json
import os
import queue
import re
import shutil
import subprocess
import sys
import tempfile
import threading
import time
from ctypes import wintypes
from pathlib import Path

import webview

APP = "To Picture"
APP_VERSION = "1.0.0"
FROZEN = getattr(sys, "frozen", False)
# в собранном exe файлы интерфейса лежат во временной папке PyInstaller (sys._MEIPASS)
HERE = Path(getattr(sys, "_MEIPASS", Path(__file__).resolve().parent))
# папка установки: рядом с exe (или рядом со скриптом при разработке)
INSTALL_DIR = Path(sys.executable).resolve().parent if FROZEN else Path(__file__).resolve().parent
BIN_DIR = INSTALL_DIR / "bin"
if BIN_DIR.is_dir():  # свои yt-dlp и ffmpeg важнее системных
    os.environ["PATH"] = str(BIN_DIR) + os.pathsep + os.environ.get("PATH", "")
OWN_YTDLP = BIN_DIR / "yt-dlp.exe"
APPDATA = Path(os.environ.get("APPDATA", Path.home()))
# портативный режим: рядом с exe лежит portable.txt — тогда всё хранится в папке data рядом,
# а в системе (AppData, реестр) программа ничего не оставляет
PORTABLE = (INSTALL_DIR / "portable.txt").exists()
DATA_DIR = INSTALL_DIR / "data" if PORTABLE else APPDATA / "ToPicture"
if os.environ.get("TOPICTURE_DATA"):  # для тестов: отдельная папка данных
    DATA_DIR = Path(os.environ["TOPICTURE_DATA"])
CONFIG = DATA_DIR / "settings.json"
STATE = DATA_DIR / "state.json"
OLD_CONFIG = DATA_DIR / "_none_" if PORTABLE else APPDATA / "YtdlCubase" / "settings.json"
NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)
UTF8_ENV = dict(os.environ, PYTHONIOENCODING="utf-8", PYTHONUTF8="1")
INFO_CACHE_TTL = 10 * 60  # данные превью годятся для скачивания 10 минут (ссылки на потоки потом протухают)
UPDATE_EVERY = 20 * 3600  # раз в сутки (с запасом)
# ошибки, которые обычно лечатся обновлением yt-dlp
OUTDATED_RE = re.compile(r"http error 403|nsig|signature|not a bot|unable to extract|"
                         r"requested format is not available|precondition|player response", re.I)

DEFAULTS = {
    "out_dir": str(Path.home() / "Videos" / "To Picture"),
    "codec": "h264",
    "crf": 16,
    "preset": "slow",
    "max_height": "best",
    "audio": "320k",
    "theme": "system",
    "lang": "auto",           # язык интерфейса: auto (как в Windows) или код из ui/locales
    # какие озвучки встроить в файл (каждая — отдельная дорожка, выбирается в плеере);
    # «ui» — язык интерфейса
    "audio_lang": "orig_ui",  # orig | orig_ui | orig_ui_en | all
    # официальные субтитры (автоматические не скачиваются никогда)
    "subs": "embed",          # off | embed | embed_srt
    "subs_lang": "ui_en",     # ui_en | ui | en | audio | all
    "encoder": "gpu",         # gpu | cpu — для H.264/H.265; без подходящей видеокарты всегда cpu
    "name_tpl": "title",      # имя файла: title | channel_title | date_title | title_id
    "sponsorblock": "off",    # off | remove | mark — реклама в видео YouTube (база SponsorBlock)
    "parallel": "on",         # on — следующее видео качается, пока перекодируется текущее
    "rate_limit": "off",      # off | 1M | 2M | 5M | 10M | 25M — байт/с
    "cookies": "off",         # off | firefox | file — вход на сайты через cookies
    "cookies_file": "",
    "proxy": "",
    "settings_open": False,
    "schema": 3,
}
AUDIO_RULES = ("orig", "orig_ui", "orig_ui_en", "all")
# шаблоны имени файла для yt-dlp (к имени потом добавляется «[1080p h264]»)
NAME_TPL = {
    "title": "%(title).170B",
    "channel_title": "%(uploader,channel,creator|)s — %(title).150B",
    "date_title": "%(upload_date>%Y-%m-%d|)s %(title).160B",
    "title_id": "%(title).150B [%(id)s]",
}
RATE_LIMITS = ("off", "1M", "2M", "5M", "10M", "25M")
SPONSOR_CATS = "sponsor,selfpromo,interaction"
# Chrome, Edge и другие браузеры на Chromium шифруют cookies ключом, который yt-dlp прочитать не может
# (и держат файл открытым), поэтому из браузеров — только Firefox; для остальных — файл cookies.txt
COOKIE_MODES = ("off", "firefox", "file")
SUBS_RULES = ("ui_en", "ui", "en", "audio", "all")

# ---- переводы: ui/locales/<код>.json — общие для интерфейса и для этого файла
LOCALES_DIR = HERE / "ui" / "locales"
_LOCALES = {}


def locale(code):
    if code not in _LOCALES:
        try:
            _LOCALES[code] = json.loads((LOCALES_DIR / f"{code}.json").read_text(encoding="utf-8"))
        except Exception:
            _LOCALES[code] = {}
    return _LOCALES[code]


def available_langs():
    out = []
    for f in sorted(LOCALES_DIR.glob("*.json")):
        out.append({"code": f.stem, "name": locale(f.stem).get("_name", f.stem)})
    return out


def system_lang():
    """Язык интерфейса Windows, сведённый к одному из переведённых."""
    try:
        buf = ctypes.create_unicode_buffer(85)
        ctypes.windll.kernel32.LCIDToLocaleName(ctypes.windll.kernel32.GetUserDefaultUILanguage(), buf, 85, 0)
        name = buf.value  # например ru-RU, zh-CN, pt-BR
    except Exception:
        name = "en-US"
    codes = [l["code"] for l in available_langs()]
    base = name.split("-")[0].lower()
    if name in codes:
        return name
    for c in codes:  # zh-TW -> zh-CN, pt-PT -> pt-BR
        if c.split("-")[0].lower() == base:
            return c
    return "en"


def resolve_lang(setting):
    codes = [l["code"] for l in available_langs()]
    return setting if setting in codes else system_lang()


def tr(lang, key, **p):
    s = locale(lang).get(key)
    if s is None:
        s = locale("en").get(key, key)
    if isinstance(s, dict):
        s = s.get("other", next(iter(s.values())))
    for k, v in p.items():
        s = s.replace("{" + k + "}", str(v))
    return s
SUBS_MODES = ("off", "embed", "embed_srt")

# ISO 639-1 -> 639-2 для метаданных дорожек в mp4/mov (плееры показывают язык по ним)
ISO3 = {
    "ru": "rus", "en": "eng", "de": "ger", "fr": "fre", "es": "spa", "it": "ita", "pt": "por",
    "ja": "jpn", "ko": "kor", "zh": "chi", "ar": "ara", "hi": "hin", "pl": "pol", "tr": "tur",
    "uk": "ukr", "vi": "vie", "id": "ind", "th": "tha", "fa": "per", "nl": "dut", "sv": "swe",
    "cs": "cze", "el": "gre", "he": "heb", "hu": "hun", "ro": "rum", "fi": "fin", "da": "dan",
    "no": "nor", "nb": "nob", "bn": "ben", "ta": "tam", "te": "tel", "mr": "mar", "ml": "mal",
    "pa": "pan", "uz": "uzb", "kk": "kaz", "be": "bel", "ms": "may", "fil": "fil", "ur": "urd",
}


def base_lang(code):
    return (code or "").split("-")[0].lower()


def iso3(code):
    b = base_lang(code)
    return ISO3.get(b, b) if b and b != "und" else "und"


def is_audio_only(info):
    """Ни одного видеопотока — например, SoundCloud или подкаст."""
    fmts = info.get("formats") or []
    if fmts:
        return all(f.get("vcodec") == "none" for f in fmts)
    return info.get("vcodec") == "none"


def video_formats(info):
    """Видеопотоки без звука в компактном виде (для выбора и для превью)."""
    out = []
    for f in info.get("formats") or []:
        if f.get("vcodec") in (None, "none") or not f.get("height") or f.get("acodec") not in (None, "none"):
            continue
        out.append({
            "id": f["format_id"], "h": f["height"], "fps": round(f.get("fps") or 0),
            "vc": (f.get("vcodec") or "").split(".")[0],
            "hdr": (f.get("dynamic_range") or "SDR") != "SDR",
            "https": (f.get("protocol") or "").startswith("http") and "m3u8" not in (f.get("protocol") or ""),
            "tbr": f.get("tbr") or 0,
        })
    return out


def select_video(vfmts, max_height, codec, allow_copy=True):
    """Какой видеопоток качать. Возвращает (формат или None, copy_video).
    - «Без перекодирования» — лучший поток как есть (включая HDR).
    - Иначе — лучший SDR (HDR при переводе в обычный H.264 выцветает).
    - Для H.264: если YouTube отдаёт avc1 того же разрешения и fps — берём его и не перекодируем."""
    if not vfmts:
        return None, False
    lim = None if max_height in (None, "", "best") else int(max_height)
    pool = [f for f in vfmts if not lim or f["h"] <= lim] or vfmts
    key = lambda f: (f["h"], f["fps"], f["https"], f["tbr"])
    if codec == "copy":
        return max(pool, key=key), False
    sdr = [f for f in pool if not f["hdr"]] or pool
    best = max(sdr, key=key)
    if codec == "h264" and allow_copy:
        avc = [f for f in sdr if f["vc"].startswith("avc1") and f["h"] == best["h"] and f["fps"] == best["fps"]]
        if avc:
            return max(avc, key=key), True
    return best, False


def audio_tracks(info, prefer_aac=False):
    """Озвучки видео: по одной лучшей дорожке на язык (без DRC — сжатой динамики)."""
    aud = [f for f in info.get("formats") or []
           if f.get("vcodec") in (None, "none") and f.get("acodec") not in (None, "none")]
    by_lang = {}
    for f in aud:
        lang = f.get("language") or "und"
        note = f.get("format_note") or ""
        key = ("drc" not in note.lower(),
               # при копировании H.264 берём AAC — тогда и звук не перекодируется
               prefer_aac and (f.get("acodec") or "").startswith("mp4a"),
               f.get("abr") or f.get("tbr") or 0)
        cur = by_lang.get(lang)
        if cur is None or key > cur[0]:
            by_lang[lang] = (key, f)
    tracks = []
    for lang, (_, f) in by_lang.items():
        note = (f.get("format_note") or "").lower()
        tracks.append({
            "lang": lang, "id": f["format_id"],
            "original": (f.get("language_preference") or 0) >= 10 or "original" in note,
            "auto": "auto" in note,  # автоперевод YouTube
        })
    tracks.sort(key=lambda t: (not t["original"], t["lang"]))
    return tracks


def pick_audio(tracks, rule, ui="en"):
    """Дорожки для встраивания. rule — правило из настроек или список языков,
    выбранный для конкретной ссылки. Оригинал всегда первый (он же включён по умолчанию)."""
    if len(tracks) <= 1:
        return list(tracks)
    if rule == "all":
        return list(tracks)
    orig = next((t for t in tracks if t["original"]), tracks[0])
    if isinstance(rule, list):
        want = [base_lang(x) for x in rule]
        out = [t for t in tracks if base_lang(t["lang"]) in want]
        out.sort(key=lambda t: (not t["original"], want.index(base_lang(t["lang"]))))
        return out or [orig]
    ui = base_lang(ui)
    extra = {"orig": [], "orig_ui": [ui], "orig_ui_en": [ui, "en"]}.get(rule, [ui])
    out = [orig]
    for lang in extra:
        t = next((t for t in tracks if base_lang(t["lang"]) == lang), None)
        if t and t not in out:
            out.append(t)
    return out


def official_subs(info):
    return sorted(k for k in (info.get("subtitles") or {}) if k != "live_chat")


def pick_subs(available, rule, audio_langs, orig_lang, ui="en"):
    """Коды официальных субтитров для встраивания — по одному на язык."""
    def one(lang):
        m = [k for k in available if base_lang(k) == lang]
        return sorted(m, key=lambda k: (k != lang, k))[:1]  # en, затем en-US / en-GB
    if rule == "all":
        return list(available)
    if rule == "audio":
        langs = [base_lang(x) for x in audio_langs] or [base_lang(orig_lang)]
    else:
        ui = base_lang(ui)
        langs = {"ui_en": [ui, "en"], "ui": [ui], "en": ["en"]}.get(rule, [ui, "en"])
    out = []
    for lang in dict.fromkeys(l for l in langs if l):
        out += one(lang)
    return out


# ---- кодировщики
CPU_THREADS = max(2, min(8, os.cpu_count() or 4))  # больше 8 потоков x264 почти не ускоряют, а память растёт
BELOW_NORMAL = 0x00004000  # приоритет ниже обычного: компьютер не тормозит во время перекодирования
GPU_FAMILIES = [
    ("NVIDIA", "NVENC", {"h264": "h264_nvenc", "h265": "hevc_nvenc"}),
    ("AMD", "AMF", {"h264": "h264_amf", "h265": "hevc_amf"}),
    ("Intel", "QSV", {"h264": "h264_qsv", "h265": "hevc_qsv"}),
]


def detect_gpu():
    """Первый кодировщик видеокарты, который реально работает (пробное кодирование 0,2 с)."""
    for vendor, short, enc in GPU_FAMILIES:
        try:
            r = run_quiet(["ffmpeg", "-hide_banner", "-loglevel", "error", "-f", "lavfi",
                           "-i", "color=black:s=640x360:d=0.2", "-c:v", enc["h264"], "-f", "null", "-"], 20)
        except Exception:
            continue
        if r.returncode == 0:
            return {"vendor": vendor, "short": short, **enc}
    return None


def encoder_args(codec, o, gpu):
    """Аргументы видеокодировщика. gpu — результат detect_gpu() или None (тогда процессор)."""
    crf, preset = int(o["crf"]), o["preset"]
    if gpu:
        enc = gpu[codec]
        if gpu["vendor"] == "NVIDIA":
            p = {"ultrafast": "p1", "superfast": "p1", "veryfast": "p2", "faster": "p3", "fast": "p3",
                 "medium": "p4", "slow": "p4", "slower": "p5", "veryslow": "p6"}.get(preset, "p4")
            return ["-c:v", enc, "-preset", p, "-tune", "hq", "-rc", "vbr", "-cq", str(crf), "-b:v", "0"]
        if gpu["vendor"] == "AMD":
            q = "speed" if preset in ("ultrafast", "superfast", "veryfast", "faster") else \
                "balanced" if preset in ("fast", "medium") else "quality"
            return ["-c:v", enc, "-quality", q, "-rc", "cqp", "-qp_i", str(crf), "-qp_p", str(crf + 2)]
        qp = {"ultrafast": "veryfast", "superfast": "veryfast"}.get(preset, preset)
        return ["-c:v", enc, "-preset", qp, "-global_quality", str(crf)]
    if codec == "h265":
        return ["-c:v", "libx265", "-preset", preset, "-crf", str(crf),
                "-x265-params", f"pools={CPU_THREADS}:rc-lookahead=20:log-level=error"]
    return ["-c:v", "libx264", "-preset", preset, "-crf", str(crf),
            "-threads", str(CPU_THREADS), "-x264-params", "rc-lookahead=20"]


# id -> (ffmpeg vcodec, acodec, контейнер, доп. аргументы, суффикс в имени файла)
CODECS = {
    "h264": ("libx264", "aac", "mp4", ["-pix_fmt", "yuv420p"], "h264"),
    "h265": ("libx265", "aac", "mp4", ["-pix_fmt", "yuv420p", "-tag:v", "hvc1"], "hevc"),
    "prores": ("prores_ks", "pcm_s24le", "mov", ["-profile:v", "2", "-pix_fmt", "yuv422p10le"], "prores"),
    "copy": (None, None, None, [], None),
}
# только звук: id -> (ffmpeg-кодек, расширение)
AUDIO_OUT = {"mp3": ("libmp3lame", "mp3"), "flac": ("flac", "flac"), "m4a": ("aac", "m4a")}

ERROR_HINTS = [
    (r"could not copy .*cookie database", "py.err.cookieLocked"),
    (r"failed to decrypt|app.bound|dpapi", "py.err.cookieDecrypt"),
    (r"could not find .*cookies database|could not find firefox", "py.err.cookieNoBrowser"),
    (r"cookies? file .*(not exist|no such)|\[errno 2\].*cookies", "py.err.cookieNoFile"),
    (r"unable to connect to proxy|proxyerror|socks|tunnel connection failed", "py.err.proxy"),
    (r"private video", "py.err.private"),
    (r"sign in to confirm your age|age.restricted", "py.err.age"),
    (r"not a bot", "py.err.bot"),
    (r"members.only|join this channel", "py.err.members"),
    (r"authentication is required|only works when logged.in|login required|requires? (a )?log.?in|"
     r"logged.in|use --cookies|account credentials", "py.err.login"),
    (r"http error 404|: not found\.|video (is )?unavailable|is not available|has been removed|does not exist",
     "py.err.unavailable"),
    (r"no video could be found|no video formats found|there is no video|no media found", "py.err.noVideo"),
    (r"requested format is not available", "py.err.noFormat"),
    (r"is not a valid url|unsupported url", "py.err.badUrl"),
    (r"premieres in|live event will begin", "py.err.premiere"),
    (r"http error 403", "py.err.http403"),
    (r"getaddrinfo|timed out|connection (refused|reset|aborted)|network is unreachable|unable to download webpage",
     "py.err.network"),
]


LOGIN_RE = re.compile(r"private video|sign in to confirm|age.restricted|not a bot|members.only|join this channel|"
                      r"authentication is required|logged.in|login required|requires? (a )?log.?in|use --cookies|"
                      r"account credentials", re.I)


def friendly_error(text, lang="en"):
    low = text.lower()
    for pat, key in ERROR_HINTS:
        if re.search(pat, low):
            return tr(lang, key)
    return re.sub(r"^ERROR:\s*(\[[^\]]+\]\s*)?([\w-]+:\s*)?", "", text).strip() or tr(lang, "py.err.unknown")


# ---------------------------------------------------------------- settings
def load_settings():
    data = dict(DEFAULTS)
    for p in (CONFIG, OLD_CONFIG):
        try:
            raw = json.loads(p.read_text(encoding="utf-8"))
        except Exception:
            continue
        if p == OLD_CONFIG:  # перенос со старой версии
            raw = {"out_dir": raw.get("out_dir"), "crf": raw.get("crf"), "preset": raw.get("preset")}
        data.update({k: v for k, v in raw.items() if k in DEFAULTS and v is not None})
        break
    if data.get("schema", 1) < 2:
        # 1.4: озвучки и субтитры по умолчанию встраиваются в файл
        data.update(audio_lang="orig_ui", subs="embed", schema=2)
    if data.get("schema", 1) < 3:
        # 1.7: «русская» -> «язык интерфейса» (для русского интерфейса поведение то же)
        data["audio_lang"] = {"orig_ru": "orig_ui", "orig_ru_en": "orig_ui_en"}.get(data["audio_lang"], data["audio_lang"])
        data["subs_lang"] = {"ru_en": "ui_en", "ru": "ui"}.get(data["subs_lang"], data["subs_lang"])
        data["schema"] = 3
    if data["audio_lang"] not in AUDIO_RULES:
        data["audio_lang"] = "orig_ui"
    if data["subs_lang"] not in SUBS_RULES:
        data["subs_lang"] = "ui_en"
    if data["subs"] == "file":
        data["subs"] = "embed_srt"
    if data["subs"] not in SUBS_MODES:
        data["subs"] = "embed"
    if data["codec"] not in CODECS and data["codec"] not in AUDIO_OUT:
        data["codec"] = "h264"
    if data["name_tpl"] not in NAME_TPL:
        data["name_tpl"] = "title"
    if data["sponsorblock"] not in ("off", "remove", "mark"):
        data["sponsorblock"] = "off"
    if data["parallel"] not in ("on", "off"):
        data["parallel"] = "on"
    if data["rate_limit"] not in RATE_LIMITS:
        data["rate_limit"] = "off"
    if data["cookies"] not in COOKIE_MODES:
        data["cookies"] = "off"
    return data


def save_settings(data):
    try:
        CONFIG.parent.mkdir(parents=True, exist_ok=True)
        CONFIG.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
    except Exception:
        pass


# ---------------------------------------------------------------- clipboard (Win32)
def _clip_api():
    u, k = ctypes.windll.user32, ctypes.windll.kernel32
    u.GetClipboardData.restype = wintypes.HANDLE
    u.SetClipboardData.argtypes = [wintypes.UINT, wintypes.HANDLE]
    u.SetClipboardData.restype = wintypes.HANDLE
    k.GlobalLock.argtypes = [wintypes.HGLOBAL]
    k.GlobalLock.restype = wintypes.LPVOID
    k.GlobalUnlock.argtypes = [wintypes.HGLOBAL]
    k.GlobalAlloc.argtypes = [wintypes.UINT, ctypes.c_size_t]
    k.GlobalAlloc.restype = wintypes.HGLOBAL
    return u, k


def clipboard_get():
    u, k = _clip_api()
    if not u.OpenClipboard(None):
        return ""
    try:
        h = u.GetClipboardData(13)  # CF_UNICODETEXT
        if not h:
            return ""
        p = k.GlobalLock(h)
        try:
            return ctypes.wstring_at(p)
        finally:
            k.GlobalUnlock(h)
    finally:
        u.CloseClipboard()


def clipboard_set(text):
    u, k = _clip_api()
    data = (text + "\0").encode("utf-16-le")
    if not u.OpenClipboard(None):
        return False
    try:
        u.EmptyClipboard()
        h = k.GlobalAlloc(0x0002, len(data))  # GMEM_MOVEABLE
        p = k.GlobalLock(h)
        ctypes.memmove(p, data, len(data))
        k.GlobalUnlock(h)
        u.SetClipboardData(13, h)
        return True
    finally:
        u.CloseClipboard()


# ---------------------------------------------------------------- helpers
def run_quiet(args, timeout=60):
    return subprocess.run(args, capture_output=True, creationflags=NO_WINDOW, timeout=timeout, env=UTF8_ENV)


def tool_versions():
    parts = []
    try:
        parts.append("yt-dlp " + run_quiet(["yt-dlp", "--version"], 20).stdout.decode().strip())
    except Exception:
        parts.append("yt-dlp —")
    try:
        first = run_quiet(["ffmpeg", "-version"], 20).stdout.decode(errors="replace").splitlines()[0]
        m = re.search(r"version\s+(\S+)", first)
        parts.append("ffmpeg " + (m.group(1)[:20] if m else "?"))
    except Exception:
        parts.append("ffmpeg —")
    return " · ".join(parts)


def probe(path):
    """Длительность, высота, fps, кадры, видеокодек."""
    try:
        r = run_quiet(["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
                       "stream=codec_name,height,r_frame_rate,nb_frames:format=duration",
                       "-of", "json", str(path)], 30)
        j = json.loads(r.stdout.decode("utf-8", errors="replace"))
        s = (j.get("streams") or [{}])[0]
        dur = float(j.get("format", {}).get("duration") or 0) or None
        num, _, den = (s.get("r_frame_rate") or "0/1").partition("/")
        fps = float(num) / float(den or 1) if float(den or 1) else 0
        a = run_quiet(["ffprobe", "-v", "error", "-select_streams", "a", "-show_entries", "stream=codec_name",
                       "-of", "csv=p=0", str(path)], 30).stdout.decode(errors="replace").split()
        return {"duration": dur, "height": s.get("height"), "fps": fps,
                "frames": int(dur * fps) if dur and fps else None, "vcodec": s.get("codec_name"), "acodecs": a}
    except Exception:
        return {"duration": None, "height": None, "fps": 0, "frames": None, "vcodec": None, "acodecs": []}


def fetch_thumb(url, referer):
    """Обложка как data: URI (до 3 МБ), с Referer страницы видео. None — если не вышло."""
    import base64, urllib.request
    try:
        req = urllib.request.Request(url, headers={
            "User-Agent": "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) "
                          "Chrome/124.0 Safari/537.36",
            "Referer": referer})
        with urllib.request.urlopen(req, timeout=10) as r:
            ctype = (r.headers.get("Content-Type") or "").split(";")[0].strip()
            data = r.read(3 * 1024 * 1024 + 1)
        if len(data) > 3 * 1024 * 1024 or not data:
            return None
        if not ctype.startswith("image/"):
            ctype = "image/webp" if data[8:12] == b"WEBP" else "image/png" if data[:4] == b"\x89PNG" else "image/jpeg"
        return f"data:{ctype};base64," + base64.b64encode(data).decode()
    except Exception:
        return None


class CookiesMissing(Exception):
    pass


def net_args(o, tmpdir=None, browser=None):
    """Вход на сайты (cookies) и прокси — для каждого вызова yt-dlp.
    browser — cookies.txt, присланный расширением (используется, только если вход в параметрах не выбран)."""
    a = []
    mode = o.get("cookies")
    if browser and mode in (None, "off") and tmpdir:
        f = Path(tmpdir) / "browser-cookies.txt"
        f.write_text(browser, encoding="utf-8")
        a += ["--cookies", str(f)]
    elif mode == "firefox":
        a += ["--cookies-from-browser", "firefox"]
    elif mode == "file":
        src = Path(o.get("cookies_file") or "")
        if not o.get("cookies_file") or not src.is_file():
            # без этой проверки yt-dlp падает с трассировкой вместо понятной ошибки
            raise CookiesMissing()
        # yt-dlp дописывает cookies обратно в файл — даём ему копию, файл пользователя не трогаем
        if tmpdir and src.is_file():
            dst = Path(tmpdir) / "cookies.txt"
            try:
                shutil.copyfile(src, dst)
                src = dst
            except OSError:
                pass
        a += ["--cookies", str(src)]
    if (o.get("proxy") or "").strip():
        a += ["--proxy", o["proxy"].strip()]
    return a


def safe_name(s, limit=120):
    """Имя папки из названия плейлиста: без символов, запрещённых в Windows."""
    s = re.sub(r'[<>:"/\\|?*\x00-\x1f]', "_", s or "").strip().rstrip(". ")
    return s[:limit].rstrip(". ")


def trim_in(trim):
    return ["-ss", f"{trim[0]:.3f}"] if trim and trim[0] else []


def trim_out(trim):
    return ["-t", f"{trim[1] - (trim[0] or 0):.3f}"] if trim and trim[1] else []


def trim_duration(trim, dur):
    if not trim:
        return dur
    end = trim[1] or dur
    return max(0.0, end - (trim[0] or 0)) if end else None


def trim_tag(trim):
    """(1.20–3.45) — в имени файла фрагмента (двоеточие в Windows запрещено)."""
    def f(s):
        s = int(round(s))
        h, m, ss = s // 3600, s % 3600 // 60, s % 60
        return f"{h}.{m:02d}.{ss:02d}" if h else f"{m}.{ss:02d}"
    a, b = trim
    return f"({f(a or 0)}–{f(b) if b else 'end'})"


def num(v):
    try:
        return float(v)
    except (TypeError, ValueError):
        return None


def unique_path(p: Path) -> Path:
    if not p.exists():
        return p
    return p.with_name(f"{p.stem}_{time.strftime('%Y%m%d_%H%M%S')}{p.suffix}")


def quality_tag(height, fps):
    if not height:
        return ""
    return f"{height}p{round(fps) if fps and fps > 31 else ''}"


def load_state():
    try:
        return json.loads(STATE.read_text(encoding="utf-8"))
    except Exception:
        return {}


def save_state(data):
    try:
        STATE.parent.mkdir(parents=True, exist_ok=True)
        STATE.write_text(json.dumps(data, ensure_ascii=False, indent=2), encoding="utf-8")
    except Exception:
        pass


# ---------------------------------------------------------------- yt-dlp self-update
class Updater:
    """Обновляет собственный bin/yt-dlp.exe: раз в сутки при запуске и сразу,
    если загрузка упала с ошибкой, похожей на «yt-dlp устарел»."""

    def __init__(self, api):
        self.api = api
        self.lock = threading.Lock()
        self.idle = threading.Event()
        self.idle.set()

    @property
    def enabled(self):
        return OWN_YTDLP.exists()

    def check(self, force=False):
        """True, если yt-dlp действительно обновился."""
        if not self.enabled:
            return False
        with self.lock:
            state = load_state()
            if not force and time.time() - state.get("ytdlp_checked", 0) < UPDATE_EVERY:
                return False
            self.idle.clear()
            try:
                L = self.api._lang
                self.api.push({"kind": "log", "line": tr(L, "py.updCheck"), "level": "n"})
                r = run_quiet([str(OWN_YTDLP), "-U"], 240)
                out = (r.stdout + r.stderr).decode("utf-8", errors="replace")
                state["ytdlp_checked"] = time.time()
                save_state(state)
                updated = bool(re.search(r"Updated yt-dlp to", out))
                failed = r.returncode != 0 and not updated
                version = run_quiet([str(OWN_YTDLP), "--version"], 30).stdout.decode().strip()
                if updated:
                    msg = tr(L, "py.updDone", v=version)
                elif failed:
                    last = [l for l in out.strip().splitlines() if l.strip()]
                    msg = tr(L, "py.updFail", e=(f" — {last[-1]}" if last else ""))
                else:
                    msg = tr(L, "py.updLatest", v=version)
                self.api.push({"kind": "ytdlp", "version": version, "updated": updated,
                               "line": msg, "level": "e" if failed else "n"})
                return updated
            except Exception as e:  # noqa: BLE001
                self.api.push({"kind": "log", "line": tr(self.api._lang, "py.updErr", e=e), "level": "e"})
                return False
            finally:
                self.idle.set()


# ---------------------------------------------------------------- preview data
def site_of(info):
    # домен понятнее внутренних имён yt-dlp («TwitchClips»): vk.com, clips.twitch.tv
    return re.sub(r"^(www|m)\.", "", info.get("webpage_url_domain") or "") or info.get("extractor_key") or ""


def preview_data(info, url, fetch=True):
    """Карточка превью по ответу yt-dlp -J (её же показываем для видео из плейлиста во время загрузки)."""
    vids = [f for f in info.get("formats") or [] if f.get("vcodec") not in (None, "none") and f.get("height")]
    best = max(vids, key=lambda f: (f["height"], f.get("fps") or 0, f.get("tbr") or 0), default=None)
    heights = sorted({f["height"] for f in vids})
    thumb = info.get("thumbnail")
    is_yt = "youtube" in (info.get("extractor") or "").lower()
    if info.get("id") and is_yt:
        thumb = f"https://i.ytimg.com/vi/{info['id']}/hqdefault.jpg"
        for t in info.get("thumbnails") or []:
            if "maxresdefault.jpg" in (t.get("url") or ""):
                thumb = t["url"]
    elif thumb and fetch:
        # многие сайты не отдают обложку без «своего» Referer — загружаем её сами и передаём как data:
        thumb = fetch_thumb(thumb, info.get("webpage_url") or url) or thumb
    return {
        "ok": True,
        "title": info.get("title") or info.get("id") or "",
        "channel": info.get("uploader") or info.get("channel") or info.get("creator") or "",
        "duration": info.get("duration"),
        "views": info.get("view_count"),
        "date": info.get("upload_date"),
        "thumb": thumb,
        "height": best and best["height"],
        "fps": best and best.get("fps"),
        "vcodec": best and (best.get("vcodec") or "").split(".")[0],
        "heights": heights,
        "vfmts": [{k: f[k] for k in ("h", "fps", "vc", "hdr", "https", "tbr")} for f in video_formats(info)],
        "tracks": [{"lang": t["lang"], "original": t["original"], "auto": t["auto"]} for t in audio_tracks(info)],
        "subs": official_subs(info),
        "auto_subs": bool(info.get("automatic_captions")),
        "language": info.get("language"),
        "audio_only": is_audio_only(info),
        "site": site_of(info),
        "youtube": is_yt,
    }


def playlist_entries(pl):
    ents = [e for e in pl.get("entries") or [] if e]
    # канал YouTube приходит вкладками «Видео», «Трансляции», «Shorts» — берём первую (это «Видео»)
    if ents and all(e.get("_type") == "playlist" for e in ents):
        return playlist_entries(ents[0])
    out = []
    for e in ents:
        u = e.get("url") or e.get("webpage_url")
        if not u:
            continue
        if not re.match(r"https?://", u) and e.get("ie_key") == "Youtube":
            u = f"https://www.youtube.com/watch?v={u}"
        if not re.match(r"https?://", u):
            continue
        out.append({"url": u, "title": e.get("title") or "", "duration": e.get("duration")})
    return out


def playlist_data(info, url, lang):
    entries = playlist_entries(info)
    if not entries:
        return {"ok": False, "error": tr(lang, "py.err.emptyPlaylist"), "raw": "empty playlist"}
    is_yt = "youtube" in (info.get("extractor") or "").lower()
    thumbs = [t.get("url") for t in info.get("thumbnails") or [] if t.get("url")]
    first = (info.get("entries") or [{}])[0] or {}
    if not thumbs:
        thumbs = [t.get("url") for t in first.get("thumbnails") or [] if t.get("url")]
    thumb = thumbs[-1] if thumbs else None
    if is_yt and entries and "watch?v=" in entries[0]["url"]:
        vid = entries[0]["url"].split("watch?v=")[1].split("&")[0]
        thumb = f"https://i.ytimg.com/vi/{vid}/hqdefault.jpg"
    elif thumb:
        thumb = fetch_thumb(thumb, info.get("webpage_url") or url) or thumb
    return {
        "ok": True, "playlist": True,
        "title": info.get("title") or info.get("id") or "",
        "channel": info.get("uploader") or info.get("channel") or "",
        "site": site_of(info), "youtube": is_yt, "thumb": thumb,
        "entries": entries, "count": len(entries),
        "duration": sum(e["duration"] or 0 for e in entries) or None,
    }


# ---------------------------------------------------------------- pause (Win32)
class _PE32(ctypes.Structure):
    _fields_ = [("dwSize", wintypes.DWORD), ("cntUsage", wintypes.DWORD), ("th32ProcessID", wintypes.DWORD),
                ("th32DefaultHeapID", ctypes.c_size_t), ("th32ModuleID", wintypes.DWORD),
                ("cntThreads", wintypes.DWORD), ("th32ParentProcessID", wintypes.DWORD),
                ("pcPriClassBase", ctypes.c_long), ("dwFlags", wintypes.DWORD), ("szExeFile", ctypes.c_wchar * 260)]


def process_tree(pid):
    """pid и все его потомки (yt-dlp.exe запускает себя второй раз, а потом ffmpeg)."""
    k = ctypes.windll.kernel32
    k.CreateToolhelp32Snapshot.restype = wintypes.HANDLE
    snap = k.CreateToolhelp32Snapshot(2, 0)  # TH32CS_SNAPPROCESS
    kids = {}
    try:
        pe = _PE32()
        pe.dwSize = ctypes.sizeof(_PE32)
        ok = k.Process32FirstW(snap, ctypes.byref(pe))
        while ok:
            kids.setdefault(pe.th32ParentProcessID, []).append(pe.th32ProcessID)
            ok = k.Process32NextW(snap, ctypes.byref(pe))
    finally:
        k.CloseHandle(snap)
    out, todo = [], [pid]
    while todo:
        p = todo.pop()
        if p in out:
            continue
        out.append(p)
        todo += kids.get(p, [])
    return out


def suspend_tree(pid, suspend=True):
    """Пауза: процессы замораживаются целиком (загрузка и перекодирование продолжаются с того же места)."""
    k, nt = ctypes.windll.kernel32, ctypes.windll.ntdll
    k.OpenProcess.restype = wintypes.HANDLE
    for p in process_tree(pid):
        h = k.OpenProcess(0x0800, False, p)  # PROCESS_SUSPEND_RESUME
        if h:
            (nt.NtSuspendProcess if suspend else nt.NtResumeProcess)(wintypes.HANDLE(h))
            k.CloseHandle(wintypes.HANDLE(h))


# ---------------------------------------------------------------- worker
class Cancelled(Exception):
    pass


class DownloadError(RuntimeError):
    def __init__(self, message, raw=""):
        super().__init__(message)
        self.raw = raw


class Task:
    """Одна ссылка в очереди (для плейлиста — одно его видео)."""

    def __init__(self, index, url, folder="", prefix="", trim=None, audio=None):
        self.index, self.url, self.folder, self.prefix = index, url, folder, prefix
        self.trim = trim    # (начало, конец или None) в секундах — только этот фрагмент
        self.audio = audio  # выбор озвучки для этой ссылки
        self.tmp = None
        self.info_path = None
        self.audio_mode = False
        self.srt_saved = []
        self.browser = False  # повтор со входом из браузера (cookies от расширения)

    def cleanup(self):
        if self.tmp:
            shutil.rmtree(self.tmp, ignore_errors=True)
            self.tmp = None


class Job(threading.Thread):
    """Очередь загрузок. Скачивание и перекодирование идут в разных потоках: пока ffmpeg
    перекодирует одно видео, следующее уже скачивается (сеть и процессор не ждут друг друга)."""

    def __init__(self, api, items, opts, overrides=None):
        super().__init__(daemon=True)
        self.api, self.opts = api, opts
        overrides = overrides or {}
        self.tasks = []
        for i, it in enumerate(items):
            it = {"url": it} if isinstance(it, str) else it
            ov = overrides.get(it["url"]) or {}
            trim = ov.get("trim")
            trim = (num(trim[0]) or 0.0, num(trim[1])) if trim else None
            self.tasks.append(Task(i, it["url"], safe_name(it.get("folder") or ""), it.get("prefix") or "",
                                   trim, ov.get("audio")))
        self.lang = resolve_lang(opts.get("lang", "auto"))
        self.procs = {}   # "dl" / "tc" -> Popen
        self.cancelled = threading.Event()
        self.paused = False
        self._resumed = threading.Event()
        self._resumed.set()
        self._plock = threading.Lock()
        self._slock = threading.Lock()
        self._last_emit = {}
        self.stats = {"saved": 0, "bytes": 0, "failed": 0}

    def emit(self, kind, **data):
        self.api.push({"kind": kind, **data})

    def emit_progress(self, force=False, **data):
        now = time.monotonic()
        key = data.get("stage")
        if force or now - self._last_emit.get(key, 0) >= 0.4:
            self._last_emit[key] = now
            self.emit("progress", **data)

    def t(self, key, **p):
        return tr(self.lang, key, **p)

    def log(self, line, level="n"):
        self.emit("log", line=line, level=level)

    def count(self, key, n=1):
        with self._slock:
            self.stats[key] += n

    # ---- pause / cancel
    def set_paused(self, on):
        with self._plock:
            if on == self.paused or self.cancelled.is_set():
                return
            self.paused = on
            (self._resumed.clear if on else self._resumed.set)()
            for p in list(self.procs.values()):
                if p and p.poll() is None:
                    try:
                        suspend_tree(p.pid, on)
                    except Exception:
                        pass
        self.emit("paused", on=on)

    def wait_resumed(self):
        while not self._resumed.wait(0.25):
            if self.cancelled.is_set():
                break
        if self.cancelled.is_set():
            raise Cancelled()

    def cancel(self):
        self.cancelled.set()
        self._resumed.set()
        for p in list(self.procs.values()):
            if p and p.poll() is None:
                subprocess.run(["taskkill", "/F", "/T", "/PID", str(p.pid)],
                               capture_output=True, creationflags=NO_WINDOW)

    def spawn(self, slot, args, **kw):
        self.wait_resumed()
        p = subprocess.Popen(args, stdin=subprocess.DEVNULL, creationflags=NO_WINDOW | BELOW_NORMAL,
                             env=UTF8_ENV, **kw)
        with self._plock:
            self.procs[slot] = p
            if self.paused:
                suspend_tree(p.pid, True)
        if self.cancelled.is_set():
            self.cancel()
        return p

    def run_proc(self, args, on_line, slot):
        self.log("$ " + " ".join(f'"{a}"' if " " in a else a for a in args), "c")
        p = self.spawn(slot, args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        for raw in p.stdout:
            line = raw.decode("utf-8", errors="replace").rstrip()
            if line:
                on_line(line)
        code = p.wait()
        self.procs.pop(slot, None)
        if self.cancelled.is_set():
            raise Cancelled()
        return code

    # ---- queue
    def net(self, task):
        return net_args(self.opts, task.tmp, self.api.browser_cookies(task.url) if task.browser else None)

    def attempt(self, task):
        try:
            return self._attempt(task)
        except DownloadError as e:
            # сайт требует вход, а в параметрах он не настроен — пробуем cookies, присланные расширением
            if task.browser or self.opts.get("cookies", "off") != "off" or not LOGIN_RE.search(e.raw) \
                    or not self.api.browser_cookies(task.url):
                raise
            self.log(self.t("py.browserLogin"))
            task.cleanup()
            task.browser = True
            self.api._info_cache.pop(task.url, None)
            return self._attempt(task)

    def _attempt(self, task):
        try:
            return self.prepare(task)
        except DownloadError as e:
            if re.search(r"http error 403", e.raw, re.I):
                # TikTok и другие иногда отказывают при частых запросах — ждём и пробуем ещё раз с нуля
                self.log(self.t("py.retry403"))
                task.cleanup()
                self.api._info_cache.pop(task.url, None)
                for _ in range(8):
                    if self.cancelled.is_set():
                        raise Cancelled()
                    time.sleep(0.5)
                try:
                    return self.prepare(task)
                except DownloadError as e2:
                    e = e2
            # частая причина падений — устаревший yt-dlp: обновляем и пробуем ещё раз
            if not (self.api._updater.enabled and OUTDATED_RE.search(e.raw)):
                raise e
            self.log(self.t("py.outdated"))
            task.cleanup()
            if not self.api._updater.check(force=True):
                raise e
            return self.prepare(task)

    def download_stage(self, task):
        """Получить данные и скачать. None — ошибка или отмена (о них уже сообщено)."""
        self.emit("item", index=task.index, total=len(self.tasks))
        try:
            return self.attempt(task)
        except Cancelled:
            self.emit("item_cancelled", index=task.index, stage="download")
        except Exception as e:  # noqa: BLE001
            self.count("failed")
            msg = str(e) or self.t("py.err.unknown")
            self.log(msg, "e")
            self.emit("item_error", index=task.index, total=len(self.tasks), message=msg, stage="download")
        task.cleanup()
        return None

    def finish_stage(self, task, prep):
        try:
            if self.cancelled.is_set():
                raise Cancelled()
            self.emit("tc_start", index=task.index)
            path = self.finalize(task, prep)
            size = path.stat().st_size
            self.count("saved")
            self.count("bytes", size)
            self.emit("item_done", index=task.index, path=str(path), size=size, srt=task.srt_saved)
        except Cancelled:
            self.emit("item_cancelled", index=task.index, stage="transcode")
        except Exception as e:  # noqa: BLE001
            self.count("failed")
            msg = str(e) or self.t("py.err.unknown")
            self.log(msg, "e")
            self.emit("item_error", index=task.index, total=len(self.tasks), message=msg, stage="transcode")
        finally:
            task.cleanup()

    def run(self):
        t0 = time.monotonic()
        if not self.api._updater.idle.is_set():
            self.log(self.t("py.waitUpdate"))
            self.api._updater.idle.wait(300)
        if self.opts.get("parallel", "on") == "on" and len(self.tasks) > 1:
            self._run_pipeline()
        else:
            for task in self.tasks:
                if self.cancelled.is_set():
                    break
                prep = self.download_stage(task)
                if prep is not None:
                    self.finish_stage(task, prep)
        s = self.stats
        self.emit("finished", saved=s["saved"], failed=s["failed"], total=len(self.tasks), bytes=s["bytes"],
                  elapsed=time.monotonic() - t0, cancelled=self.cancelled.is_set())

    def _run_pipeline(self):
        q = queue.Queue()
        # не больше одного скачанного видео «в запасе»: временные файлы не копятся на диске
        slots = threading.Semaphore(2)

        def producer():
            try:
                for task in self.tasks:
                    while not slots.acquire(timeout=0.25):
                        if self.cancelled.is_set():
                            break
                    else:
                        if self.cancelled.is_set():
                            slots.release()
                            break
                        prep = self.download_stage(task)
                        if prep is None:
                            slots.release()
                        else:
                            q.put((task, prep))
                        continue
                    break
            finally:
                q.put(None)

        th = threading.Thread(target=producer, daemon=True)
        th.start()
        while True:
            item = q.get()
            if item is None:
                break
            try:
                self.finish_stage(*item)
            finally:
                slots.release()
        th.join()

    # ---- download half
    def prepare(self, task):
        o = self.opts
        task.tmp = Path(tempfile.mkdtemp(prefix="topicture_"))
        try:
            net_args(o, task.tmp)
        except CookiesMissing:
            raise DownloadError(self.t("py.err.cookieNoFile"), "cookies file missing")
        task.audio_mode = False
        meta = self.fetch_info(task)
        if task.folder:  # видео из плейлиста — превью показываем по уже полученным данным
            try:
                self.emit("meta", index=task.index, data=preview_data(meta, task.url, fetch=False))
            except Exception:
                pass

        if o["codec"] in AUDIO_OUT or is_audio_only(meta):
            return self.prepare_audio(task, meta)
        # видео: SDR вместо HDR; H.264 от YouTube — без перекодирования (кроме фрагмента: его режем точно по кадру)
        vpick, video_copy = select_video(video_formats(meta), o["max_height"], o["codec"], allow_copy=not task.trim)
        if video_copy:
            self.log(self.t("py.copyVideo", q=f"{vpick['h']}p{vpick['fps']}"))
        elif vpick and o["codec"] != "copy":
            self.log(self.t("py.videoPick", q=f"{vpick['h']}p{vpick['fps']}", vc=vpick["vc"], hdr=" (HDR)" if vpick["hdr"] else ""))

        # озвучка: правило из настроек или выбор для конкретного видео
        tracks = audio_tracks(meta, prefer_aac=video_copy)
        rule = task.audio or o["audio_lang"]
        chosen = pick_audio(tracks, rule, self.lang) if len(tracks) > 1 else []
        if chosen:
            self.log(self.t("py.tracks", n=len(chosen), total=len(tracks), list=", ".join(
                t["lang"] + (self.t("py.orig") if t["original"] else "") for t in chosen)))

        # субтитры: только официальные, автоматические не берём
        subs_keys = []
        if o["subs"] != "off":
            avail = official_subs(meta)
            subs_keys = pick_subs(avail, o["subs_lang"], [t["lang"] for t in chosen], meta.get("language"), self.lang)
            if subs_keys:
                self.log(self.t("py.subs", list=", ".join(subs_keys)))
            elif avail:
                self.log(self.t("py.subsMissing", list=", ".join(avail)))
            else:
                self.log(self.t("py.subsNone"))
        # план загрузки — чтобы прогресс показывал только реальные шаги
        self.emit("plan", index=task.index, subs=bool(subs_keys), audio_n=max(1, len(chosen)), copy=video_copy)

        src, sub_files = self.download(task, chosen, subs_keys, vpick["id"] if vpick else None)
        return {"kind": "video", "src": src, "subs": sub_files, "chosen": chosen, "tracks": tracks,
                "video_copy": video_copy}

    def prepare_audio(self, task, meta):
        o = self.opts
        source_audio = is_audio_only(meta)
        self.log(self.t("py.audioOnly") if source_audio else self.t("py.audioOut", f=o["codec"].upper()))
        task.audio_mode = True
        tracks = audio_tracks(meta)
        rule = task.audio or o["audio_lang"]
        # в аудиофайле одна дорожка — первая из выбранных (оригинал, если он отмечен)
        chosen = pick_audio(tracks, rule, self.lang)[:1] if len(tracks) > 1 else []
        self.emit("plan", index=task.index, subs=False, audio_n=1, audio_only=True)
        fmt = (chosen[0]["id"] + "/" if chosen else "") + "ba[acodec^=mp4a]/ba[acodec=mp3]/ba/b"
        src, _ = self.download(task, [], [], fmt=fmt)
        lang = (chosen or tracks or [{"lang": None}])[0]["lang"]
        return {"kind": "audio", "src": src, "lang": lang,
                "title": meta.get("title") or "", "artist": meta.get("uploader") or meta.get("channel") or ""}

    def fetch_info(self, task):
        """yt-dlp -J во временный файл; загрузка потом идёт по нему (--load-info-json),
        чтобы не ходить на сайт дважды."""
        self.emit_progress(force=True, index=task.index, stage="download", step="video", pct=None,
                           note=self.t("py.fetching"))
        path = task.tmp / "info.json"
        # данные уже получены для превью — используем их: меньше запросов к сайту (TikTok за частые даёт 403)
        cached = self.api._info_cache.get(task.url)
        if cached and time.time() - cached[0] < INFO_CACHE_TTL:
            if len(cached) > 2 and cached[2]:  # превью получено со входом из браузера — и качаем так же
                task.browser = True
            path.write_bytes(cached[1])
            task.info_path = path
            return json.loads(cached[1].decode("utf-8", errors="replace"))
        args = ["yt-dlp", "-J", "--no-playlist", "--no-warnings", *self.net(task), "--", task.url]
        self.log("$ " + " ".join(args), "c")
        with open(path, "wb") as out:
            p = self.spawn("dl", args, stdout=out, stderr=subprocess.PIPE)
            err = p.stderr.read().decode("utf-8", errors="replace")
            code = p.wait()
            self.procs.pop("dl", None)
        if self.cancelled.is_set():
            raise Cancelled()
        errors = [l for l in err.splitlines() if l.startswith("ERROR")]
        for l in err.splitlines():
            if l.strip():
                self.log(l, "e" if l.startswith("ERROR") else "n")
        if code != 0:
            raise DownloadError(friendly_error(errors[-1], self.lang) if errors else self.t("py.ytdlpExit", code=code),
                                "\n".join(errors))
        task.info_path = path
        return json.loads(path.read_text(encoding="utf-8", errors="replace"))

    def download(self, task, chosen, subs_keys, video_id=None, fmt=None):
        o = self.opts
        h = o["max_height"]
        lim = "" if h == "best" else f"[height<={int(h)}]"
        v = video_id or f"bv*{lim}"
        # запасные варианты: не все сайты сообщают размер кадра, у некоторых видео и звук — один файл
        tail = f"bv*{lim}+ba/b{lim}/bv*+ba/b"
        if fmt:
            pass
        elif chosen:
            fmt = f"{v}+" + "+".join(t["id"] for t in chosen) + f"/{v}+ba/{tail}"
        else:
            fmt = f"{v}+ba[acodec^=mp4a]/{v}+ba/{v}/{tail}" if video_id else tail
        name = NAME_TPL.get(o.get("name_tpl"), NAME_TPL["title"])
        args = ["yt-dlp", "--load-info-json", str(task.info_path), "-f", fmt, *self.net(task),
                "--merge-output-format", "mkv", "--newline", "--color", "never", "--progress-template",
                "download:PROG %(progress.downloaded_bytes)s %(progress.total_bytes)s "
                "%(progress.total_bytes_estimate)s %(progress.speed)s %(progress.eta)s",
                "-o", str(task.tmp / f"{name}.%(ext)s")]
        if o.get("rate_limit") in RATE_LIMITS[1:]:
            args += ["-r", o["rate_limit"]]
        # SponsorBlock (только YouTube): с фрагментом не сочетается — сдвинулось бы время
        sb = o.get("sponsorblock")
        if sb == "remove" and not task.trim:
            args += ["--sponsorblock-remove", SPONSOR_CATS]
        elif sb == "mark":
            args += ["--sponsorblock-mark", SPONSOR_CATS, "--embed-chapters"]
        if len(chosen) > 1:
            args.append("--audio-multistreams")
        if subs_keys:
            args += ["--write-subs", "--sub-langs", ",".join(subs_keys), "--convert-subs", "srt"]
        t0 = time.monotonic()
        st = {"step": -1, "errors": [], "media": 0}
        n_audio = max(1, len(chosen))
        idx = task.index

        def on_line(line):
            if line.startswith("PROG "):
                if st["step"] == -1:
                    return  # субтитры качаются мгновенно — их прогресс не показываем
                p = line.split()
                done, tot, est, speed, eta = (num(x) for x in p[1:6])
                tot = tot or est
                self.emit_progress(index=idx, stage="download", step=st["step"], done=done, total=tot, speed=speed,
                                   eta=eta, pct=(done / tot * 100) if (done is not None and tot) else None,
                                   audio_i=max(1, st["media"] - 1), audio_n=n_audio)
                return
            self.log(line, "e" if line.startswith("ERROR") else "n")
            if line.startswith("ERROR"):
                st["errors"].append(line)
            elif "[download] Destination:" in line:
                if re.search(r"\.(vtt|srt|ttml|srv\d|json3)$", line, re.I):
                    st["step"] = -1
                    self.emit_progress(force=True, index=idx, stage="download", step="subs", pct=None,
                                       note=self.t("py.subsNote"))
                else:
                    st["media"] += 1
                    st["step"] = "audio" if task.audio_mode or st["media"] > 1 else "video"
                    self.emit_progress(force=True, index=idx, stage="download", step=st["step"], pct=0,
                                       audio_i=max(1, st["media"] - 1), audio_n=n_audio)
            elif line.startswith("[Merger]"):
                self.emit_progress(force=True, index=idx, stage="download", step="merge", pct=100)
            elif line.startswith("[SponsorBlock]") or line.startswith("[ModifyChapters]"):
                self.emit_progress(force=True, index=idx, stage="download", step="merge", pct=100,
                                   note=self.t("py.sponsorNote"))

        code = self.run_proc(args, on_line, "dl")
        files = [f for f in task.tmp.iterdir() if f.is_file() and not f.name.endswith((".part", ".ytdl"))
                 and f.suffix.lower() in (".mkv", ".webm", ".mp4", ".mov", ".m4a", ".mp3", ".opus", ".ogg",
                                          ".aac", ".flac", ".wav", ".flv", ".ts")]
        if code != 0 or not files:
            if not st["errors"] and code == 0:  # yt-dlp отработал, но видео в публикации нет (например, картинки)
                raise DownloadError(self.t("py.err.noVideo"), "no video")
            raise DownloadError(friendly_error(st["errors"][-1], self.lang) if st["errors"] else
                                self.t("py.ytdlpExit", code=code), "\n".join(st["errors"]))
        src = max(files, key=lambda f: f.stat().st_size)
        subs = []
        for f in sorted(task.tmp.glob("*.srt")):
            suf = f.suffixes
            if len(suf) >= 2 and f.stat().st_size > 0:
                subs.append((suf[-2][1:], f))
        self.emit("stage_done", index=idx, stage="download", size=src.stat().st_size,
                  elapsed=time.monotonic() - t0, merged=st["media"] > 1)
        return src, subs

    # ---- transcode half
    def out_dir(self, task):
        d = Path(self.opts["out_dir"])
        if task.folder:
            d = d / task.folder
        d.mkdir(parents=True, exist_ok=True)
        return d

    def stem(self, task, src):
        # «Канал — Название» без канала, «Дата Название» без даты: убираем висящие разделители
        return task.prefix + (re.sub(r"^[\s—–-]+", "", src.stem).strip() or src.stem)

    def finalize(self, task, p):
        if p["kind"] == "audio":
            return self.finalize_audio(task, p)
        o = self.opts
        out_dir = self.out_dir(task)
        src, sub_files, chosen, tracks, video_copy = p["src"], p["subs"], p["chosen"], p["tracks"], p["video_copy"]
        info = probe(src)
        vcodec, acodec, ext, extra, tag = CODECS[o["codec"]]
        stem = self.stem(task, src)

        q = quality_tag(info["height"], info["fps"])
        a_tag = ""
        if len(chosen) > 3:
            a_tag = " multi"
        elif len(chosen) > 1 or (chosen and not chosen[0]["original"]):
            a_tag = " " + "+".join(base_lang(t["lang"]) for t in chosen)
        if chosen:
            audio_meta = [(t["lang"], self.t("py.trackOriginal") if t["original"] else self.t("py.trackAuto") if t["auto"] else "")
                          for t in chosen]
        else:  # одна дорожка — всё равно подпишем язык, если сайт его знает
            audio_meta = [(tracks[0]["lang"], "")] if tracks and tracks[0]["lang"] != "und" else []
        embed = sub_files if o["subs"] in ("embed", "embed_srt") else []
        cut = " " + trim_tag(task.trim) if task.trim else ""

        if vcodec is None:
            orig = info["vcodec"] or "orig"
            if embed or len(chosen) > 1 or audio_meta or task.trim:
                # перепаковка без перекодирования: вшить субтитры, подписать дорожки, вырезать фрагмент
                dest = unique_path(out_dir / f"{stem}{cut} [{q} {orig}{a_tag}].mkv")
                self.convert(task, src, dest, info, "copy", "copy", [], embed, audio_meta)
            else:
                dest = unique_path(out_dir / f"{stem} [{q} {orig}{a_tag}]{src.suffix}")
                shutil.move(str(src), dest)
        else:
            if embed:
                # со встроенными субтитрами — MKV: в MP4 первые субтитры всегда «включены по умолчанию»,
                # а в MKV они выключены, пока их не выберут в плеере
                ext = "mkv"
                extra = [a for i, a in enumerate(extra)
                         if a != "-tag:v" and (i == 0 or extra[i - 1] != "-tag:v")]  # hvc1 — только для mp4
            dest = unique_path(out_dir / f"{stem}{cut} [{q} {tag}{a_tag}].{ext}")
            self.convert(task, src, dest, info, vcodec, acodec, extra, embed, audio_meta, video_copy=video_copy)

        if embed:
            self.log(self.t("py.embedded", list=", ".join(l for l, _ in embed)))
        task.srt_saved = []
        if o["subs"] == "embed_srt":
            for lang, f in sub_files:
                target = unique_path(dest.with_name(f"{dest.stem}.{lang}.srt"))
                if task.trim:  # субтитры к фрагменту — тоже со сдвигом времени
                    self.run_proc(["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", *trim_in(task.trim),
                                   "-i", str(f), *trim_out(task.trim), str(target)], lambda l: self.log(l, "e"), "tc")
                else:
                    shutil.move(str(f), target)
                task.srt_saved.append(str(target))
                self.log(self.t("py.srtSaved", name=target.name))
        return dest

    def finalize_audio(self, task, p):
        o = self.opts
        src = p["src"]
        out_dir = self.out_dir(task)
        stem = self.stem(task, src) + (" " + trim_tag(task.trim) if task.trim else "")
        a = probe(src)
        first = (a.get("acodecs") or [""])[0]
        target = o["codec"] if o["codec"] in AUDIO_OUT else None
        copy = False
        if target == "flac":
            # блоки одного размера: иначе после обрезки AAC первый короткий кадр ломает кодировщик FLAC
            ext, args = "flac", ["-af", "asetnsamples=n=4096:p=0", "-c:a", "flac"]
        elif target == "mp3":
            copy = first == "mp3"
            ext, args = "mp3", ["-c:a", "copy"] if copy else ["-c:a", "libmp3lame", "-b:a", o["audio"]]
        elif first in ("aac", "mp3"):  # M4A или «звук как есть» — AAC/MP3 с сайта не пережимаем
            copy = True
            ext, args = ("m4a" if first == "aac" else "mp3"), ["-c:a", "copy"]
        else:
            ext, args = "m4a", ["-c:a", "aac", "-b:a", o["audio"]]
        if copy:
            self.emit("tc_mode", index=task.index, copy=True)
        dest = unique_path(out_dir / f"{stem}.{ext}")
        meta = ["-map_metadata", "-1", "-metadata", f"title={p['title']}"]
        if p["artist"]:
            meta += ["-metadata", f"artist={p['artist']}"]
        if p.get("lang") and p["lang"] != "und":
            meta += ["-metadata:s:a:0", f"language={iso3(p['lang'])}"]
        cmd = ["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1",
               *trim_in(task.trim), "-i", str(src), "-vn", "-map", "0:a:0", *trim_out(task.trim), *args, *meta]
        if ext == "m4a":
            cmd += ["-movflags", "+faststart"]
        cmd.append(str(dest))
        dur = trim_duration(task.trim, a["duration"])
        self.ffmpeg(task, cmd, dest, dur, None)
        task.srt_saved = []
        return dest

    def convert(self, task, src, dest, info, vcodec, acodec, extra, subs=(), audio_meta=(), video_copy=False):
        if video_copy:
            self.emit("tc_mode", index=task.index, copy=True)
            return self._convert(task, src, dest, info, vcodec, acodec, extra, subs, audio_meta, None, video_copy=True)
        codec = {"libx264": "h264", "libx265": "h265"}.get(vcodec)
        gpu = self.api._gpu_for_job() if codec and self.opts.get("encoder") == "gpu" else None
        if gpu:
            self.log(self.t("py.gpuUse", vendor=gpu["vendor"], enc=gpu[codec]))
            try:
                return self._convert(task, src, dest, info, vcodec, acodec, extra, subs, audio_meta, gpu)
            except Cancelled:
                raise
            except RuntimeError as e:
                self.log(self.t("py.gpuFail", e=e), "e")
                self.emit_progress(force=True, index=task.index, stage="transcode", pct=0, frame=0,
                                   frames=info["frames"], speed=None, eta=None)
        return self._convert(task, src, dest, info, vcodec, acodec, extra, subs, audio_meta, None)

    def _convert(self, task, src, dest, info, vcodec, acodec, extra, subs, audio_meta, gpu, video_copy=False):
        o = self.opts
        codec = {"libx264": "h264", "libx265": "h265"}.get(vcodec)
        cut_in = trim_in(task.trim)
        args = ["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1",
                *cut_in, "-i", str(src)]
        for _, f in subs:
            args += [*cut_in, "-i", str(f)]
        args += ["-map", "0:v:0", "-map", "0:a?"]
        for i in range(len(subs)):
            args += ["-map", f"{i + 1}:0"]
        args += trim_out(task.trim)
        if task.trim:
            args += ["-map_chapters", "-1"]
        if vcodec == "copy":
            args += ["-c:v", "copy", "-c:a", "copy"]
        elif video_copy:
            # видео уже H.264 — только перепаковка; звук AAC тоже копируем, иначе пережимаем в AAC (это быстро)
            args += ["-c:v", "copy"]
            if info.get("acodecs") and all(c == "aac" for c in info["acodecs"]):
                args += ["-c:a", "copy"]
            else:
                args += ["-c:a", "aac", "-b:a", o["audio"]]
        else:
            if codec:
                args += encoder_args(codec, o, gpu)
            else:
                args += ["-c:v", vcodec]
            args += extra + ["-c:a", acodec]
            if acodec == "aac":
                args += ["-b:a", o["audio"]]
        # подписываем дорожки языком — плееры и монтажки показывают его в списке
        for i, (lang, title) in enumerate(audio_meta):
            args += [f"-metadata:s:a:{i}", f"language={iso3(lang)}",
                     f"-disposition:a:{i}", "default" if i == 0 else "0"]
            if title:
                args += [f"-metadata:s:a:{i}", f"title={title}"]
        if subs:
            args += ["-c:s", "srt" if dest.suffix == ".mkv" else "mov_text"]
            for i, (lang, _) in enumerate(subs):
                args += [f"-metadata:s:s:{i}", f"language={iso3(lang)}", f"-disposition:s:{i}", "0"]
        if dest.suffix == ".mp4":
            args += ["-movflags", "+faststart"]
        args.append(str(dest))
        dur = trim_duration(task.trim, info["duration"])
        frames = int(dur * info["fps"]) if dur and info["fps"] else info["frames"]
        self.ffmpeg(task, args, dest, dur, frames)

    def ffmpeg(self, task, args, dest, dur, frames):
        t0 = time.monotonic()
        cur = {"frame": 0, "speed": None}
        errors = []
        idx = task.index

        def on_line(line):
            k, sep, v = line.partition("=")
            if not sep or " " in k:
                errors.append(line)
                self.log(line, "e")
                return
            if k == "frame":
                cur["frame"] = int(num(v) or 0)
            elif k == "speed":
                cur["speed"] = num(v.rstrip("x"))
            elif k == "out_time_us" and dur:
                sec = (num(v) or 0) / 1e6
                pct = max(0.0, min(100.0, sec / dur * 100))
                el = time.monotonic() - t0
                eta = el * (100 - pct) / pct if pct > 0.5 else None
                self.emit_progress(index=idx, stage="transcode", pct=pct, frame=cur["frame"], frames=frames,
                                   speed=cur["speed"], eta=eta)
            elif k == "progress" and v == "end":
                self.log(f"frame={cur['frame']} speed={cur['speed']}x", "n")

        code = self.run_proc(args, on_line, "tc")
        if code != 0 or not dest.exists() or dest.stat().st_size == 0:
            dest.unlink(missing_ok=True)
            raise RuntimeError(self.t("py.ffmpegFail") + (f": {errors[-1]}" if errors else "."))
        self.emit("stage_done", index=idx, stage="transcode", size=dest.stat().st_size, elapsed=time.monotonic() - t0)


# ---------------------------------------------------------------- browser extension bridge
# Расширение (папка extension) отправляет ссылку через native messaging: браузер запускает
# «To Picture.exe chrome-extension://…/», тот кладёт ссылку в папку inbox и будит открытое окно
# (через локальный сокет) или запускает программу.
HOST_NAME = "com.topicture.app"
EXT_ID = "kefllcapjmcgojghjpaofcpoigeiocgi"  # постоянный: задан ключом в extension/manifest.json
EXT_DIR = INSTALL_DIR / "extension"
INBOX = DATA_DIR / "inbox"
IPC_FILE = DATA_DIR / "ipc.json"
HOST_MANIFEST = DATA_DIR / "native-host.json"
HOST_KEYS = [  # браузеры на Chromium ищут описание связи каждый в своём разделе реестра
    r"Software\Google\Chrome\NativeMessagingHosts",
    r"Software\Microsoft\Edge\NativeMessagingHosts",
    r"Software\Chromium\NativeMessagingHosts",
    r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
    r"Software\Vivaldi\NativeMessagingHosts",
    r"Software\Yandex\YandexBrowser\NativeMessagingHosts",
]


def host_executable():
    """Что запускает браузер. В собранной программе — сам exe; при разработке — bat с python."""
    if FROZEN:
        return Path(sys.executable)
    bat = DATA_DIR / "native-host.bat"
    py = Path(sys.executable).with_name("python.exe")
    env = f'@set "TOPICTURE_DATA={os.environ["TOPICTURE_DATA"]}"\r\n' if os.environ.get("TOPICTURE_DATA") else ""
    bat.write_text(env + f'@"{py}" "{Path(__file__).resolve()}" %*\r\n', encoding="utf-8")
    return bat


def register_extension():
    import winreg
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    HOST_MANIFEST.write_text(json.dumps({
        "name": HOST_NAME, "description": "To Picture", "path": str(host_executable()), "type": "stdio",
        "allowed_origins": [f"chrome-extension://{EXT_ID}/"],
    }, indent=2), encoding="utf-8")
    for key in HOST_KEYS:
        with winreg.CreateKey(winreg.HKEY_CURRENT_USER, key + "\\" + HOST_NAME) as k:
            winreg.SetValueEx(k, "", 0, winreg.REG_SZ, str(HOST_MANIFEST))
    return True


def unregister_extension():
    import winreg
    for key in HOST_KEYS:
        try:
            winreg.DeleteKey(winreg.HKEY_CURRENT_USER, key + "\\" + HOST_NAME)
        except OSError:
            pass
    HOST_MANIFEST.unlink(missing_ok=True)
    (DATA_DIR / "native-host.bat").unlink(missing_ok=True)


def extension_registered():
    """Связь настроена и ведёт именно к этой копии программы."""
    import winreg
    try:
        with winreg.OpenKey(winreg.HKEY_CURRENT_USER, HOST_KEYS[0] + "\\" + HOST_NAME) as k:
            path = Path(winreg.QueryValueEx(k, "")[0])
        target = json.loads(path.read_text(encoding="utf-8")).get("path", "")
    except (OSError, ValueError):
        return False
    if FROZEN:
        return os.path.normcase(target) == os.path.normcase(sys.executable)
    return Path(target).exists()


def site_key(url):
    """youtube.com для www.youtube.com, m.youtube.com и youtu.be."""
    from urllib.parse import urlparse
    try:
        h = (urlparse(url).hostname or "").lower()
    except ValueError:
        return ""
    h = re.sub(r"^(www|m)\.", "", h)
    return "youtube.com" if h == "youtu.be" else h


def netscape_cookies(cookies):
    """Список cookies от расширения -> формат cookies.txt, который понимает yt-dlp."""
    lines = ["# Netscape HTTP Cookie File"]
    for c in cookies or []:
        try:
            dom = c["domain"]
            sub = not c.get("hostOnly")
            if sub and not dom.startswith("."):
                dom = "." + dom
            lines.append("\t".join([("#HttpOnly_" if c.get("httpOnly") else "") + dom, "TRUE" if sub else "FALSE",
                                    c.get("path") or "/", "TRUE" if c.get("secure") else "FALSE",
                                    str(int(c.get("expirationDate") or 0)), c["name"], c["value"]]))
        except (KeyError, TypeError, ValueError):
            continue
    return "\n".join(lines) + "\n"


# ---- сторона браузера: короткоживущий процесс, который запускает Chrome/Edge
def _std_io():
    k = ctypes.windll.kernel32
    k.GetStdHandle.restype = wintypes.HANDLE
    hin, hout = k.GetStdHandle(-10), k.GetStdHandle(-11)

    def read(n):
        out = b""
        while len(out) < n:
            buf = ctypes.create_string_buffer(n - len(out))
            got = wintypes.DWORD()
            if not k.ReadFile(wintypes.HANDLE(hin), buf, n - len(out), ctypes.byref(got), None) or not got.value:
                break
            out += buf.raw[:got.value]
        return out

    def write(obj):
        data = json.dumps(obj).encode("utf-8")
        data = len(data).to_bytes(4, "little") + data
        done = wintypes.DWORD()
        k.WriteFile(wintypes.HANDLE(hout), data, len(data), ctypes.byref(done), None)
        k.FlushFileBuffers(wintypes.HANDLE(hout))

    return read, write


def wake_app():
    """Разбудить открытое окно: оно заберёт ссылки из inbox. False — окна нет."""
    import socket
    try:
        ipc = json.loads(IPC_FILE.read_text(encoding="utf-8"))
        with socket.create_connection(("127.0.0.1", int(ipc["port"])), timeout=1.5) as s:
            s.sendall((ipc["token"] + "\n").encode())
            return s.recv(8) == b"ok"
    except (OSError, ValueError, KeyError):
        return False


def launch_app():
    args = [sys.executable] if FROZEN else [str(Path(sys.executable).with_name("pythonw.exe")), str(Path(__file__).resolve())]
    flags = 0x00000008 | 0x00000200  # DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP
    # без сброса новая копия exe унаследует служебные переменные PyInstaller и возьмёт временную папку
    # этого процесса, которая исчезнет, когда он завершится, — программа упадёт при запуске
    env = dict(os.environ, PYINSTALLER_RESET_ENVIRONMENT="1")
    for extra in (0x01000000, 0):    # CREATE_BREAKAWAY_FROM_JOB: браузер не закроет программу вместе с собой
        try:
            subprocess.Popen(args, creationflags=flags | extra, close_fds=True, cwd=str(INSTALL_DIR), env=env,
                             stdin=subprocess.DEVNULL, stdout=subprocess.DEVNULL, stderr=subprocess.DEVNULL)
            return True
        except OSError:
            continue
    return False


def host_message(m):
    if not isinstance(m, dict):
        return {"ok": False, "error": "bad message"}
    if m.get("type") == "ping":
        return {"ok": True, "version": APP_VERSION}
    url = m.get("url") or ""
    if m.get("type") != "add" or not re.match(r"https?://", url):
        return {"ok": False, "error": "bad url"}
    INBOX.mkdir(parents=True, exist_ok=True)
    name = f"{time.time():.6f}-{os.getpid()}.json"
    tmp = INBOX / (name + ".tmp")
    tmp.write_text(json.dumps({"url": url, "title": m.get("title") or "", "cookies": m.get("cookies") or [],
                               "time": time.time()}, ensure_ascii=False), encoding="utf-8")
    tmp.replace(INBOX / name)
    st = load_state()
    st["ext_seen"] = time.time()
    save_state(st)
    if wake_app():
        return {"ok": True, "launched": False}
    return {"ok": launch_app(), "launched": True}


def native_host():
    read, write = _std_io()
    while True:
        head = read(4)
        if len(head) < 4:
            break
        n = int.from_bytes(head, "little")
        try:
            msg = json.loads(read(n).decode("utf-8"))
            write(host_message(msg))
        except Exception as e:  # noqa: BLE001
            write({"ok": False, "error": str(e)})


# ---------------------------------------------------------------- JS API
class Api:
    def __init__(self):
        self._window = None
        self._job = None
        self._lock = threading.Lock()
        self._updater = Updater(self)
        self._gpu = None
        self._info_cache = {}  # url -> (время, JSON от yt-dlp -J, со входом из браузера) из превью
        self._browser_cookies = {}  # сайт -> cookies.txt, присланный расширением (только в памяти)
        self._inbox_lock = threading.Lock()
        self._lang = resolve_lang(load_settings().get("lang", "auto"))
        self._gpu_ready = threading.Event()

    # pywebview отдаёт публичные атрибуты в JS, поэтому окно держим в "приватном" поле
    def attach(self, window):
        self._window = window

    def push(self, ev):
        w = self._window
        if w:
            try:
                w.evaluate_js(f"window.app && app.onEvent({json.dumps(ev, ensure_ascii=False)})")
            except Exception:
                pass

    def init(self):
        missing = [t for t in ("yt-dlp", "ffmpeg", "ffprobe") if not shutil.which(t)]
        settings = load_settings()
        self._lang = resolve_lang(settings.get("lang", "auto"))
        return {"settings": settings, "missing": missing, "lang": self._lang, "system_lang": system_lang(),
                "strings": locale(self._lang), "fallback": locale("en"), "langs": available_langs()}

    def _detect_gpu(self):
        try:
            self._gpu = detect_gpu()
        finally:
            self._gpu_ready.set()

    def _gpu_for_job(self):
        self._gpu_ready.wait(30)
        return self._gpu

    def gpu_info(self):
        """Для интерфейса: какая видеокарта умеет кодировать (или None)."""
        self._gpu_ready.wait(30)
        g = self._gpu
        return {"vendor": g["vendor"], "short": g["short"]} if g else None

    def versions(self):
        """Версии для журнала; заодно в фоне проверяем обновление yt-dlp."""
        threading.Thread(target=self._updater.check, daemon=True).start()
        if not self._gpu_ready.is_set() and not getattr(self, "_gpu_started", False):
            self._gpu_started = True
            threading.Thread(target=self._detect_gpu, daemon=True).start()
        return f"To Picture {APP_VERSION}{' portable' if PORTABLE else ''} · " + tool_versions()

    def save_settings(self, data):
        clean = dict(DEFAULTS)
        clean.update({k: v for k, v in (data or {}).items() if k in DEFAULTS})
        save_settings(clean)
        self._lang = resolve_lang(clean.get("lang", "auto"))
        return True

    def preview(self, url):
        s = load_settings()
        with tempfile.TemporaryDirectory(prefix="topicture_pv_") as tmp:
            try:
                net = net_args(s, tmp)
            except CookiesMissing:
                return {"ok": False, "error": tr(self._lang, "py.err.cookieNoFile"), "raw": "cookies file missing"}
            # --flat-playlist: у плейлиста и канала — только список видео, без данных каждого (быстро)
            r = run_quiet(["yt-dlp", "-J", "--flat-playlist", "--no-playlist", "--no-warnings",
                           *net, "--", url], 180)
            browser = False
            bc = self.browser_cookies(url)
            if r.returncode != 0 and bc and s.get("cookies", "off") == "off" \
                    and LOGIN_RE.search(r.stderr.decode("utf-8", errors="replace")):
                # нужен вход — пробуем cookies, которые прислало расширение вместе со ссылкой
                r = run_quiet(["yt-dlp", "-J", "--flat-playlist", "--no-playlist", "--no-warnings",
                               *net_args(s, tmp, bc), "--", url], 180)
                browser = r.returncode == 0
        if r.returncode != 0:
            lines = r.stderr.decode("utf-8", errors="replace").strip().splitlines()
            err = next((l for l in reversed(lines) if l.startswith("ERROR")), lines[-1] if lines else "")
            return {"ok": False, "error": friendly_error(err, self._lang), "raw": err}
        info = json.loads(r.stdout.decode("utf-8", errors="replace"))
        if info.get("_type") == "playlist":
            return playlist_data(info, url, self._lang)
        self._info_cache[url] = (time.time(), r.stdout, browser)
        # публикация без видео и звука (например, пост с картинками) — говорим сразу, а не при скачивании
        if not info.get("formats") and not info.get("url"):
            return {"ok": False, "error": tr(self._lang, "py.err.noVideo"), "raw": "no formats"}
        d = preview_data(info, url)
        d["browser_login"] = browser
        return d

    # ---- расширение для браузера
    def browser_cookies(self, url):
        key = site_key(url)
        for site, text in self._browser_cookies.items():
            if key == site or key.endswith("." + site) or site.endswith("." + key):
                return text
        return None

    def start_ipc(self):
        """Локальный сокет: процесс, запущенный браузером, будит это окно, и оно забирает ссылки."""
        import secrets, socket
        srv = socket.socket()
        srv.bind(("127.0.0.1", 0))
        srv.listen(4)
        token = secrets.token_hex(16)
        try:
            DATA_DIR.mkdir(parents=True, exist_ok=True)
            IPC_FILE.write_text(json.dumps({"port": srv.getsockname()[1], "token": token, "pid": os.getpid()}),
                                encoding="utf-8")
        except OSError:
            return

        def loop():
            while True:
                try:
                    c, _ = srv.accept()
                except OSError:
                    return
                with c:
                    try:
                        c.settimeout(2)
                        if c.recv(64).decode(errors="replace").strip() == token:
                            c.sendall(b"ok")
                            self.take_inbox(focus=True)
                    except OSError:
                        pass
        threading.Thread(target=loop, daemon=True).start()

    def stop_ipc(self):
        try:  # файл указывает на это окно — убираем, чтобы браузер не стучался в закрытое
            if json.loads(IPC_FILE.read_text(encoding="utf-8")).get("pid") == os.getpid():
                IPC_FILE.unlink()
        except (OSError, ValueError):
            pass

    def links_ready(self):
        """Интерфейс загрузился: забрать ссылки, пришедшие до этого (браузер запустил программу)."""
        self._links_ready = True
        self.take_inbox(focus=False)
        st = load_state()
        # установлено с расширением, но им ещё не пользовались — один раз покажем, как его подключить
        show = extension_registered() and not st.get("ext_seen") and not st.get("ext_help_shown") and EXT_DIR.is_dir()
        if show:
            st["ext_help_shown"] = True
            save_state(st)
        return {"show_help": show}

    def take_inbox(self, focus=True):
        if not getattr(self, "_links_ready", False):
            return
        with self._inbox_lock:
            got = []
            for f in sorted(INBOX.glob("*.json")) if INBOX.is_dir() else []:
                try:
                    m = json.loads(f.read_text(encoding="utf-8"))
                except (OSError, ValueError):
                    m = None
                f.unlink(missing_ok=True)  # cookies на диске не задерживаются
                if not m or time.time() - (m.get("time") or 0) > 3600:
                    continue
                if m.get("cookies"):
                    self._browser_cookies[site_key(m["url"])] = netscape_cookies(m["cookies"])
                got.append(m)
        for m in got:
            self.push({"kind": "ext_link", "url": m["url"], "title": m.get("title") or ""})
        if got and focus:
            self.focus_window()

    def focus_window(self):
        try:
            w = self._window
            w.restore()
            hwnd = int(w.native.Handle.ToInt64())
            u = ctypes.windll.user32
            if not u.SetForegroundWindow(hwnd):
                # Windows не даёт фоновому процессу забрать фокус — мигаем кнопкой на панели задач
                class FW(ctypes.Structure):
                    _fields_ = [("cbSize", wintypes.UINT), ("hwnd", wintypes.HWND), ("dwFlags", wintypes.DWORD),
                                ("uCount", wintypes.UINT), ("dwTimeout", wintypes.DWORD)]
                fw = FW(ctypes.sizeof(FW), hwnd, 0x3 | 0xC, 3, 0)  # FLASHW_ALL | FLASHW_TIMERNOFG
                u.FlashWindowEx(ctypes.byref(fw))
        except Exception:
            pass

    def ext_status(self):
        st = load_state()
        return {"registered": extension_registered(), "folder": str(EXT_DIR), "has_folder": EXT_DIR.is_dir(),
                "seen": st.get("ext_seen"), "portable": PORTABLE, "id": EXT_ID}

    def ext_register(self):
        try:
            register_extension()
        except OSError:
            return False
        return extension_registered()

    def ext_open_folder(self):
        if EXT_DIR.is_dir():
            os.startfile(EXT_DIR)
        return True

    def pause(self, on):
        job = self._job
        if job and job.is_alive():
            job.set_paused(bool(on))
        return True

    def browse_file(self, current):
        """Файл cookies.txt."""
        start = os.path.dirname(current) if current and os.path.isdir(os.path.dirname(current)) else ""
        res = self._window.create_file_dialog(webview.FileDialog.OPEN, directory=start,
                                              file_types=("cookies (*.txt)", "All files (*.*)"))
        return os.path.normpath(res[0]) if res else None

    def start(self, urls, opts, overrides=None):
        with self._lock:
            if self._job and self._job.is_alive():
                return False
            clean = dict(DEFAULTS)
            clean.update({k: v for k, v in opts.items() if k in DEFAULTS})
            save_settings(clean)
            items = [u if isinstance(u, dict) else {"url": u} for u in urls]
            items = [i for i in items if isinstance(i.get("url"), str) and i["url"]]
            self._job = Job(self, items, clean, overrides or {})
            self._job.start()
            return True

    def cancel(self):
        job = self._job
        if job and job.is_alive():
            job.cancel()
        return True

    def browse(self, current):
        res = self._window.create_file_dialog(webview.FileDialog.FOLDER,
                                              directory=current if os.path.isdir(current or "") else "")
        return os.path.normpath(res[0]) if res else None

    def show_in_folder(self, path):
        if path and os.path.exists(path):
            subprocess.Popen(["explorer", "/select,", os.path.normpath(path)])
        elif path:
            os.startfile(os.path.dirname(path))
        return True

    def open_data_dir(self):
        DATA_DIR.mkdir(parents=True, exist_ok=True)
        os.startfile(DATA_DIR)
        return True

    def app_info(self):
        return {"version": APP_VERSION, "portable": PORTABLE}

    def get_clipboard(self):
        try:
            return clipboard_get()
        except Exception:
            return ""

    def set_clipboard(self, text):
        try:
            return clipboard_set(text)
        except Exception:
            return False

    def set_title(self, title):
        if self._window:
            self._window.set_title(title)

    def set_dark(self, dark):
        """Тёмный системный заголовок окна, как у приложений Windows 11."""
        try:
            try:
                hwnd = int(self._window.native.Handle.ToInt64())
            except Exception:
                hwnd = ctypes.windll.user32.FindWindowW(None, self._window.title)
            val = ctypes.c_int(1 if dark else 0)
            for attr in (20, 19):  # DWMWA_USE_IMMERSIVE_DARK_MODE (новые / старые сборки)
                if ctypes.windll.dwmapi.DwmSetWindowAttribute(hwnd, attr, ctypes.byref(val), 4) == 0:
                    break
            # перерисовать рамку
            ctypes.windll.user32.SetWindowPos(hwnd, 0, 0, 0, 0, 0, 0x0001 | 0x0002 | 0x0004 | 0x0020)
        except Exception:
            pass

    def closing(self):
        self.stop_ipc()
        job = self._job
        if job and job.is_alive():
            job.cancel()


def system_dark():
    try:
        import winreg
        k = winreg.OpenKey(winreg.HKEY_CURRENT_USER,
                           r"Software\Microsoft\Windows\CurrentVersion\Themes\Personalize")
        return winreg.QueryValueEx(k, "AppsUseLightTheme")[0] == 0
    except Exception:
        return False


def main():
    args = sys.argv[1:]
    if any(a.startswith("chrome-extension://") for a in args):
        return native_host()  # запущено браузером по просьбе расширения
    if "--register-extension" in args:
        return register_extension()
    if "--unregister-extension" in args:
        return unregister_extension()
    api = Api()
    theme = load_settings().get("theme", "system")
    dark = theme == "dark" or (theme == "system" and system_dark())
    window = webview.create_window(
        APP, url=str(HERE / "ui" / "index.html"), js_api=api,
        width=820, height=760, min_size=(720, 640), background_color="#1E1E1D" if dark else "#F3F3F1",
        text_select=False,
    )
    api.attach(window)
    api.start_ipc()
    window.events.closing += api.closing
    try:  # своя кнопка на панели задач, а не общая с python.exe
        ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID("ToPicture.App")
    except Exception:
        pass
    icon = HERE / "icon.ico"
    storage = DATA_DIR / "webview"
    drop_stale_ui_cache(storage)
    webview.start(private_mode=False, storage_path=str(storage), http_port=free_port(),
                  icon=str(icon) if icon.exists() else None, debug="--debug" in sys.argv)


def free_port():
    """pywebview с private_mode=False всегда берёт порт 42001. Если он занят другим окном To Picture
    (например, старой версией), новое окно молча показывало ЕГО интерфейс. Берём свободный порт."""
    import socket
    with socket.socket() as s:
        s.bind(("127.0.0.1", 0))
        return s.getsockname()[1]


def drop_stale_ui_cache(storage):
    """Если файлы интерфейса изменились (обновление программы) — чистим кэш WebView2, чтобы не подхватить
    старые страницы. Настройки программы хранятся отдельно и не затрагиваются."""
    import hashlib
    h = hashlib.sha1()
    for f in [HERE / "ui" / "index.html", *sorted((HERE / "ui" / "locales").glob("*.json"))]:
        try:
            h.update(f.read_bytes())
        except OSError:
            pass
    stamp = h.hexdigest()
    marker = storage / ".ui-stamp"
    try:
        if marker.read_text(encoding="utf-8") == stamp:
            return
    except OSError:
        pass
    for sub in ("Cache", "Code Cache", "GPUCache", "Service Worker"):
        shutil.rmtree(storage / "EBWebView" / "Default" / sub, ignore_errors=True)
    try:
        storage.mkdir(parents=True, exist_ok=True)
        marker.write_text(stamp, encoding="utf-8")
    except OSError:
        pass


if __name__ == "__main__":
    main()
