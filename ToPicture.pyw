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
APP_VERSION = "1.6.1"
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
CONFIG = DATA_DIR / "settings.json"
STATE = DATA_DIR / "state.json"
OLD_CONFIG = DATA_DIR / "_none_" if PORTABLE else APPDATA / "YtdlCubase" / "settings.json"
NO_WINDOW = getattr(subprocess, "CREATE_NO_WINDOW", 0)
UTF8_ENV = dict(os.environ, PYTHONIOENCODING="utf-8", PYTHONUTF8="1")
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
    # какие озвучки встроить в файл (каждая — отдельная дорожка, выбирается в плеере)
    "audio_lang": "orig_ru",  # orig | orig_ru | orig_ru_en | all
    # официальные субтитры (автоматические не скачиваются никогда)
    "subs": "embed",          # off | embed | embed_srt
    "subs_lang": "ru_en",     # ru_en | ru | en | audio | all
    "encoder": "gpu",         # gpu | cpu — для H.264/H.265; без подходящей видеокарты всегда cpu
    "settings_open": False,
    "schema": 2,
}
AUDIO_RULES = ("orig", "orig_ru", "orig_ru_en", "all")
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


def select_video(vfmts, max_height, codec):
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
    if codec == "h264":
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


def pick_audio(tracks, rule):
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
    extra = {"orig": [], "orig_ru": ["ru"], "orig_ru_en": ["ru", "en"]}.get(rule, ["ru"])
    out = [orig]
    for lang in extra:
        t = next((t for t in tracks if base_lang(t["lang"]) == lang), None)
        if t and t not in out:
            out.append(t)
    return out


def official_subs(info):
    return sorted(k for k in (info.get("subtitles") or {}) if k != "live_chat")


def pick_subs(available, rule, audio_langs, orig_lang):
    """Коды официальных субтитров для встраивания — по одному на язык."""
    def one(lang):
        m = [k for k in available if base_lang(k) == lang]
        return sorted(m, key=lambda k: (k != lang, k))[:1]  # en, затем en-US / en-GB
    if rule == "all":
        return list(available)
    if rule == "audio":
        langs = [base_lang(x) for x in audio_langs] or [base_lang(orig_lang)]
    else:
        langs = {"ru_en": ["ru", "en"], "ru": ["ru"], "en": ["en"]}.get(rule, ["ru", "en"])
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

ERROR_HINTS = [
    (r"private video", "Это приватное видео. Нужен доступ от владельца канала."),
    (r"sign in to confirm your age|age.restricted", "Видео с возрастным ограничением — нужен вход в аккаунт."),
    (r"video (is )?unavailable|is not available|has been removed", "Видео недоступно или удалено."),
    (r"members.only|join this channel", "Видео только для спонсоров канала."),
    (r"is not a valid url|unsupported url", "Ссылка не распознана."),
    (r"premieres in|live event will begin", "Премьера ещё не началась."),
    (r"unable to download webpage|getaddrinfo|timed out|connection", "Нет связи с сайтом. Проверь интернет."),
    (r"http error 403", "Сайт отказал в доступе (403). Обнови yt-dlp: yt-dlp -U"),
]


def friendly_error(text):
    low = text.lower()
    for pat, msg in ERROR_HINTS:
        if re.search(pat, low):
            return msg
    return re.sub(r"^ERROR:\s*(\[[^\]]+\]\s*)?([\w-]+:\s*)?", "", text).strip() or "Неизвестная ошибка."


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
    if data["codec"] not in CODECS:
        data["codec"] = "h264"
    if data.get("schema", 1) < 2:
        # 1.4: озвучки и субтитры по умолчанию встраиваются в файл
        data.update(audio_lang="orig_ru", subs="embed", schema=2)
    if data["audio_lang"] not in AUDIO_RULES:
        data["audio_lang"] = "orig_ru"
    if data["subs"] == "file":
        data["subs"] = "embed_srt"
    if data["subs"] not in SUBS_MODES:
        data["subs"] = "embed"
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
        parts.append("yt-dlp не найден")
    try:
        first = run_quiet(["ffmpeg", "-version"], 20).stdout.decode(errors="replace").splitlines()[0]
        m = re.search(r"version\s+(\S+)", first)
        parts.append("ffmpeg " + (m.group(1)[:20] if m else "?"))
    except Exception:
        parts.append("ffmpeg не найден")
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
                self.api.push({"kind": "log", "line": "yt-dlp: проверка обновлений…", "level": "n"})
                r = run_quiet([str(OWN_YTDLP), "-U"], 240)
                out = (r.stdout + r.stderr).decode("utf-8", errors="replace")
                state["ytdlp_checked"] = time.time()
                save_state(state)
                updated = bool(re.search(r"Updated yt-dlp to", out))
                failed = r.returncode != 0 and not updated
                version = run_quiet([str(OWN_YTDLP), "--version"], 30).stdout.decode().strip()
                if updated:
                    msg = f"yt-dlp обновлён до {version}"
                elif failed:
                    last = [l for l in out.strip().splitlines() if l.strip()]
                    msg = "yt-dlp: не удалось обновиться" + (f" — {last[-1]}" if last else "")
                else:
                    msg = f"yt-dlp {version} — последняя версия"
                self.api.push({"kind": "ytdlp", "version": version, "updated": updated,
                               "line": msg, "level": "e" if failed else "n"})
                return updated
            except Exception as e:  # noqa: BLE001
                self.api.push({"kind": "log", "line": f"yt-dlp: ошибка обновления — {e}", "level": "e"})
                return False
            finally:
                self.idle.set()


# ---------------------------------------------------------------- worker
class Cancelled(Exception):
    pass


class DownloadError(RuntimeError):
    def __init__(self, message, raw=""):
        super().__init__(message)
        self.raw = raw


class Job(threading.Thread):
    def __init__(self, api, items, opts, overrides=None):
        super().__init__(daemon=True)
        self.api, self.items, self.opts = api, items, opts
        self.overrides = overrides or {}
        self.info_path = None
        self.proc = None
        self.cancelled = threading.Event()
        self.tmp = None
        self._last_emit = 0.0

    def emit(self, kind, **data):
        self.api.push({"kind": kind, **data})

    def emit_progress(self, force=False, **data):
        now = time.monotonic()
        if force or now - self._last_emit >= 0.4:
            self._last_emit = now
            self.emit("progress", **data)

    def log(self, line, level="n"):
        self.emit("log", line=line, level=level)

    def cancel(self):
        self.cancelled.set()
        p = self.proc
        if p and p.poll() is None:
            subprocess.run(["taskkill", "/F", "/T", "/PID", str(p.pid)],
                           capture_output=True, creationflags=NO_WINDOW)

    def run_proc(self, args, on_line):
        self.log("$ " + " ".join(f'"{a}"' if " " in a else a for a in args), "c")
        self.proc = subprocess.Popen(args, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                     stdin=subprocess.DEVNULL, creationflags=NO_WINDOW | BELOW_NORMAL,
                                     env=UTF8_ENV)
        for raw in self.proc.stdout:
            line = raw.decode("utf-8", errors="replace").rstrip()
            if line:
                on_line(line)
        code = self.proc.wait()
        self.proc = None
        if self.cancelled.is_set():
            raise Cancelled()
        return code

    # ---- main loop
    def attempt(self, url):
        try:
            return self.process(url)
        except DownloadError as e:
            # частая причина падений — устаревший yt-dlp: обновляем и пробуем ещё раз
            if not (self.api._updater.enabled and OUTDATED_RE.search(e.raw)):
                raise
            self.log("Похоже, yt-dlp устарел — обновляю и пробую ещё раз…")
            if self.tmp:
                shutil.rmtree(self.tmp, ignore_errors=True)
                self.tmp = None
            if not self.api._updater.check(force=True):
                raise
            return self.process(url)

    def run(self):
        total = len(self.items)
        t0 = time.monotonic()
        saved, saved_bytes, failed = 0, 0, 0
        if not self.api._updater.idle.is_set():
            self.log("Жду окончания обновления yt-dlp…")
            self.api._updater.idle.wait(300)
        for i, url in enumerate(self.items):
            if self.cancelled.is_set():
                break
            self.emit("item", index=i, total=total)
            try:
                path = self.attempt(url)
                size = path.stat().st_size
                saved += 1
                saved_bytes += size
                self.emit("item_done", index=i, path=str(path), size=size, srt=getattr(self, "srt_saved", []))
            except Cancelled:
                self.emit("item_cancelled", index=i)
                break
            except Exception as e:  # noqa: BLE001
                failed += 1
                msg = str(e) or "Неизвестная ошибка."
                self.log(msg, "e")
                self.emit("item_error", index=i, total=total, message=msg)
            finally:
                if self.tmp:
                    shutil.rmtree(self.tmp, ignore_errors=True)
                    self.tmp = None
        self.emit("finished", saved=saved, failed=failed, total=total, bytes=saved_bytes,
                  elapsed=time.monotonic() - t0, cancelled=self.cancelled.is_set())

    def process(self, url):
        o = self.opts
        out_dir = Path(o["out_dir"])
        out_dir.mkdir(parents=True, exist_ok=True)
        self.tmp = Path(tempfile.mkdtemp(prefix="topicture_"))
        meta = self.fetch_info(url)

        # видео: SDR вместо HDR; H.264 от YouTube — без перекодирования
        vpick, video_copy = select_video(video_formats(meta), o["max_height"], o["codec"])
        if video_copy:
            self.log(f"YouTube отдаёт H.264 {vpick['h']}p{vpick['fps']} — перекодирование не нужно")
        elif vpick and o["codec"] != "copy":
            self.log(f"Видео: {vpick['h']}p{vpick['fps']} {vpick['vc']}" + (" (HDR)" if vpick["hdr"] else ""))

        # озвучка: правило из настроек или выбор для конкретного видео
        tracks = audio_tracks(meta, prefer_aac=video_copy)
        rule = (self.overrides.get(url) or {}).get("audio") or o["audio_lang"]
        chosen = pick_audio(tracks, rule) if len(tracks) > 1 else []
        if chosen:
            self.log(f"Озвучки в файле ({len(chosen)} из {len(tracks)}): " + ", ".join(
                t["lang"] + (" — оригинал" if t["original"] else "") for t in chosen))

        # субтитры: только официальные, автоматические не берём
        subs_keys = []
        if o["subs"] != "off":
            avail = official_subs(meta)
            subs_keys = pick_subs(avail, o["subs_lang"], [t["lang"] for t in chosen], meta.get("language"))
            if subs_keys:
                self.log("Субтитры: " + ", ".join(subs_keys))
            elif avail:
                self.log("Субтитров на нужном языке нет. Есть: " + ", ".join(avail))
            else:
                self.log("Официальных субтитров у видео нет")

        src, sub_files = self.download(chosen, subs_keys, vpick["id"] if vpick else None)
        info = probe(src)
        vcodec, acodec, ext, extra, tag = CODECS[o["codec"]]

        q = quality_tag(info["height"], info["fps"])
        a_tag = ""
        if len(chosen) > 3:
            a_tag = " multi"
        elif len(chosen) > 1 or (chosen and not chosen[0]["original"]):
            a_tag = " " + "+".join(base_lang(t["lang"]) for t in chosen)
        if chosen:
            audio_meta = [(t["lang"], "Оригинал" if t["original"] else "Автоперевод" if t["auto"] else "")
                          for t in chosen]
        else:  # одна дорожка — всё равно подпишем язык, если YouTube его знает
            audio_meta = [(tracks[0]["lang"], "")] if tracks and tracks[0]["lang"] != "und" else []
        embed = sub_files if o["subs"] in ("embed", "embed_srt") else []

        if vcodec is None:
            orig = info["vcodec"] or "orig"
            if embed or len(chosen) > 1 or audio_meta:
                # перепаковка без перекодирования, чтобы вшить субтитры и подписать дорожки
                dest = unique_path(out_dir / f"{src.stem} [{q} {orig}{a_tag}].mkv")
                self.convert(src, dest, info, "copy", "copy", [], embed, audio_meta)
            else:
                dest = unique_path(out_dir / f"{src.stem} [{q} {orig}{a_tag}]{src.suffix}")
                shutil.move(str(src), dest)
        else:
            if embed:
                # со встроенными субтитрами — MKV: в MP4 первые субтитры всегда «включены по умолчанию»,
                # а в MKV они выключены, пока их не выберут в плеере
                ext = "mkv"
                extra = [a for i, a in enumerate(extra)
                         if a != "-tag:v" and (i == 0 or extra[i - 1] != "-tag:v")]  # hvc1 — только для mp4
            dest = unique_path(out_dir / f"{src.stem} [{q} {tag}{a_tag}].{ext}")
            self.convert(src, dest, info, vcodec, acodec, extra, embed, audio_meta, video_copy=video_copy)

        if embed:
            self.log("Встроены субтитры: " + ", ".join(l for l, _ in embed))
        self.srt_saved = []
        if o["subs"] == "embed_srt":
            for lang, f in sub_files:
                target = unique_path(dest.with_name(f"{dest.stem}.{lang}.srt"))
                shutil.move(str(f), target)
                self.srt_saved.append(str(target))
                self.log(f"Субтитры сохранены: {target.name}")
        return dest

    # ---- metadata
    def fetch_info(self, url):
        """yt-dlp -J во временный файл; загрузка потом идёт по нему (--load-info-json),
        чтобы не ходить на сайт дважды."""
        self.emit_progress(force=True, stage="download", step="video", pct=None, note="получение данных…")
        path = self.tmp / "info.json"
        args = ["yt-dlp", "-J", "--no-playlist", "--no-warnings", "--", url]
        self.log("$ " + " ".join(args), "c")
        with open(path, "wb") as out:
            self.proc = subprocess.Popen(args, stdout=out, stderr=subprocess.PIPE, stdin=subprocess.DEVNULL,
                                         creationflags=NO_WINDOW, env=UTF8_ENV)
            err = self.proc.stderr.read().decode("utf-8", errors="replace")
            code = self.proc.wait()
            self.proc = None
        if self.cancelled.is_set():
            raise Cancelled()
        errors = [l for l in err.splitlines() if l.startswith("ERROR")]
        for l in err.splitlines():
            if l.strip():
                self.log(l, "e" if l.startswith("ERROR") else "n")
        if code != 0:
            raise DownloadError(friendly_error(errors[-1]) if errors else f"yt-dlp завершился с кодом {code}",
                                "\n".join(errors))
        self.info_path = path
        return json.loads(path.read_text(encoding="utf-8", errors="replace"))

    # ---- download
    def download(self, chosen, subs_keys, video_id=None):
        h = self.opts["max_height"]
        lim = "" if h == "best" else f"[height<={int(h)}]"
        v = video_id or f"bv*{lim}"
        if chosen:
            fmt = f"{v}+" + "+".join(t["id"] for t in chosen) + f"/b{lim}"
        else:
            fmt = f"{v}+ba[acodec^=mp4a]/{v}+ba/b{lim}" if video_id else f"{v}+ba/b{lim}"
        args = ["yt-dlp", "--load-info-json", str(self.info_path), "-f", fmt,
                "--merge-output-format", "mkv", "--newline", "--color", "never", "--progress-template",
                "download:PROG %(progress.downloaded_bytes)s %(progress.total_bytes)s "
                "%(progress.total_bytes_estimate)s %(progress.speed)s %(progress.eta)s",
                "-o", str(self.tmp / "%(title).180B.%(ext)s")]
        if len(chosen) > 1:
            args.append("--audio-multistreams")
        if subs_keys:
            args += ["--write-subs", "--sub-langs", ",".join(subs_keys), "--convert-subs", "srt"]
        t0 = time.monotonic()
        st = {"step": -1, "errors": [], "media": 0}
        n_audio = max(1, len(chosen))

        def on_line(line):
            if line.startswith("PROG "):
                if st["step"] == -1:
                    return  # субтитры качаются мгновенно — их прогресс не показываем
                p = line.split()
                done, tot, est, speed, eta = (num(x) for x in p[1:6])
                tot = tot or est
                self.emit_progress(stage="download", step=st["step"], done=done, total=tot, speed=speed, eta=eta,
                                   pct=(done / tot * 100) if (done is not None and tot) else None,
                                   audio_i=max(1, st["media"] - 1), audio_n=n_audio)
                return
            self.log(line, "e" if line.startswith("ERROR") else "n")
            if line.startswith("ERROR"):
                st["errors"].append(line)
            elif "[download] Destination:" in line:
                if re.search(r"\.(vtt|srt|ttml|srv\d|json3)$", line, re.I):
                    st["step"] = -1
                    self.emit_progress(force=True, stage="download", step="subs", pct=None, note="субтитры…")
                else:
                    st["media"] += 1
                    st["step"] = "video" if st["media"] == 1 else "audio"
                    self.emit_progress(force=True, stage="download", step=st["step"], pct=0,
                                       audio_i=max(1, st["media"] - 1), audio_n=n_audio)
            elif line.startswith("[Merger]"):
                self.emit_progress(force=True, stage="download", step="merge", pct=100)

        code = self.run_proc(args, on_line)
        files = [f for f in self.tmp.iterdir() if f.is_file() and not f.name.endswith((".part", ".ytdl"))
                 and f.suffix.lower() in (".mkv", ".webm", ".mp4", ".mov", ".m4a")]
        if code != 0 or not files:
            raise DownloadError(friendly_error(st["errors"][-1]) if st["errors"] else
                                f"yt-dlp завершился с кодом {code}", "\n".join(st["errors"]))
        src = max(files, key=lambda f: f.stat().st_size)
        subs = []
        for f in sorted(self.tmp.glob("*.srt")):
            suf = f.suffixes
            if len(suf) >= 2 and f.stat().st_size > 0:
                subs.append((suf[-2][1:], f))
        self.emit("stage_done", stage="download", size=src.stat().st_size,
                  elapsed=time.monotonic() - t0, merged=st["media"] > 1)
        return src, subs

    # ---- transcode / remux
    def convert(self, src, dest, info, vcodec, acodec, extra, subs=(), audio_meta=(), video_copy=False):
        if video_copy:
            self.emit("tc_mode", copy=True)
            return self._convert(src, dest, info, vcodec, acodec, extra, subs, audio_meta, None, video_copy=True)
        codec = {"libx264": "h264", "libx265": "h265"}.get(vcodec)
        gpu = self.api._gpu_for_job() if codec and self.opts.get("encoder") == "gpu" else None
        if gpu:
            self.log(f"Кодирую на видеокарте {gpu['vendor']} ({gpu[codec]})")
            try:
                return self._convert(src, dest, info, vcodec, acodec, extra, subs, audio_meta, gpu)
            except Cancelled:
                raise
            except RuntimeError as e:
                self.log(f"Видеокарта не справилась ({e}) — кодирую процессором", "e")
                self.emit_progress(force=True, stage="transcode", pct=0, frame=0, frames=info["frames"], speed=None, eta=None)
        return self._convert(src, dest, info, vcodec, acodec, extra, subs, audio_meta, None)

    def _convert(self, src, dest, info, vcodec, acodec, extra, subs, audio_meta, gpu, video_copy=False):
        o = self.opts
        codec = {"libx264": "h264", "libx265": "h265"}.get(vcodec)
        args = ["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1",
                "-i", str(src)]
        for _, f in subs:
            args += ["-i", str(f)]
        args += ["-map", "0:v:0", "-map", "0:a?"]
        for i in range(len(subs)):
            args += ["-map", f"{i + 1}:0"]
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

        dur, frames = info["duration"], info["frames"]
        t0 = time.monotonic()
        cur = {"frame": 0, "speed": None}
        errors = []

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
                self.emit_progress(stage="transcode", pct=pct, frame=cur["frame"], frames=frames,
                                   speed=cur["speed"], eta=eta)
            elif k == "progress" and v == "end":
                self.log(f"frame={cur['frame']} speed={cur['speed']}x", "n")

        code = self.run_proc(args, on_line)
        if code != 0 or not dest.exists() or dest.stat().st_size == 0:
            dest.unlink(missing_ok=True)
            raise RuntimeError("ffmpeg не смог обработать файл" + (f": {errors[-1]}" if errors else "."))
        self.emit("stage_done", stage="transcode", size=dest.stat().st_size, elapsed=time.monotonic() - t0)


# ---------------------------------------------------------------- JS API
class Api:
    def __init__(self):
        self._window = None
        self._job = None
        self._lock = threading.Lock()
        self._updater = Updater(self)
        self._gpu = None
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
        return {"settings": load_settings(), "missing": missing}

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
        return True

    def preview(self, url):
        r = run_quiet(["yt-dlp", "-J", "--no-playlist", "--no-warnings", "--", url], 60)
        if r.returncode != 0:
            lines = r.stderr.decode("utf-8", errors="replace").strip().splitlines()
            err = next((l for l in reversed(lines) if l.startswith("ERROR")), lines[-1] if lines else "")
            return {"ok": False, "error": friendly_error(err), "raw": err}
        info = json.loads(r.stdout.decode("utf-8", errors="replace"))
        vids = [f for f in info.get("formats") or [] if f.get("vcodec") not in (None, "none") and f.get("height")]
        best = max(vids, key=lambda f: (f["height"], f.get("fps") or 0, f.get("tbr") or 0), default=None)
        heights = sorted({f["height"] for f in vids})
        thumb = info.get("thumbnail")
        if info.get("id") and "youtube" in (info.get("extractor") or "").lower():
            thumb = f"https://i.ytimg.com/vi/{info['id']}/hqdefault.jpg"
            for t in info.get("thumbnails") or []:
                if "maxresdefault.jpg" in (t.get("url") or ""):
                    thumb = t["url"]
        return {
            "ok": True,
            "title": info.get("title") or "Без названия",
            "channel": info.get("uploader") or info.get("channel") or "",
            "duration": info.get("duration"),
            "views": info.get("view_count"),
            "date": info.get("upload_date"),
            "thumb": thumb,
            "height": best and best["height"],
            "fps": best and best.get("fps"),
            "vcodec": best and (best.get("vcodec") or "").split(".")[0],
            "heights": heights,
            "vfmts": [{k: f[k] for k in ("h", "fps", "vc", "hdr", "https", "tbr")} for f in video_formats(info)],
            "tracks": [{"lang": t["lang"], "original": t["original"], "auto": t["auto"]}
                       for t in audio_tracks(info)],
            "subs": official_subs(info),
            "auto_subs": bool(info.get("automatic_captions")),
            "language": info.get("language"),
        }

    def start(self, urls, opts, overrides=None):
        with self._lock:
            if self._job and self._job.is_alive():
                return False
            clean = dict(DEFAULTS)
            clean.update({k: v for k, v in opts.items() if k in DEFAULTS})
            save_settings(clean)
            self._job = Job(self, list(urls), clean, overrides or {})
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
    api = Api()
    theme = load_settings().get("theme", "system")
    dark = theme == "dark" or (theme == "system" and system_dark())
    window = webview.create_window(
        APP, url=str(HERE / "ui" / "index.html"), js_api=api,
        width=820, height=760, min_size=(720, 640), background_color="#1E1E1D" if dark else "#F3F3F1",
        text_select=False,
    )
    api.attach(window)
    window.events.closing += api.closing
    try:  # своя кнопка на панели задач, а не общая с python.exe
        ctypes.windll.shell32.SetCurrentProcessExplicitAppUserModelID("ToPicture.App")
    except Exception:
        pass
    icon = HERE / "icon.ico"
    webview.start(private_mode=False, storage_path=str(DATA_DIR / "webview"),
                  icon=str(icon) if icon.exists() else None, debug="--debug" in sys.argv)


if __name__ == "__main__":
    main()
