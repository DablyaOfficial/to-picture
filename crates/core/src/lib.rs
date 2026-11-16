//! To Picture — движок: настройки, переводы, превью (yt-dlp -J), очередь загрузок и перекодирования.
//! Порт логики из ToPicture.pyw (Python-версия 1.0.0). Интерфейса здесь нет: события уходят в `EventSink`.
//!
//! КОНТРАКТ. Публичные типы и сигнатуры в этом файле согласованы между crates (core / bridge / app).
//! Менять их можно только добавлением полей/методов; переименование и удаление ломают другие части.

#![allow(clippy::collapsible_if)]

pub mod paths;    // INSTALL_DIR, DATA_DIR (portable.txt, TOPICTURE_DATA), BIN_DIR, CONFIG, STATE
pub mod i18n;     // ui/locales/*.json, встроенные в exe через include_str!; tr(), множественные формы
pub mod settings; // Settings (совместим с settings.json Python-версии, schema 3), load/save, state.json
pub mod util;     // safe_name, unique_path, trim, обложки
pub mod media;    // выбор видеопотока, озвучек, субтитров
pub mod encode;   // CODECS, AUDIO_OUT, encoder_args, detect_gpu
pub mod errors;   // ERROR_HINTS, friendly_error, LOGIN_RE, OUTDATED_RE
pub mod net;      // net_args (cookies, прокси), netscape_cookies
pub mod procs;    // запуск без окна, дерево процессов, пауза/остановка
pub mod updater;  // обновление своего yt-dlp
pub mod preview;  // yt-dlp -J -> карточка видео / плейлиста
pub mod job;      // очередь: скачивание + перекодирование

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

pub const APP: &str = "To Picture";
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");

pub use settings::Settings;

/// Куда движок отправляет события. Вызывается из рабочих потоков.
pub type EventSink = Arc<dyn Fn(Event) + Send + Sync>;

/// События очереди — те же, что Python отправлял в app.onEvent (поле kind = имя варианта в snake_case).
#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum Event {
    Log { line: String, level: LogLevel },
    Paused { on: bool },
    /// Началась загрузка ссылки index из total.
    Item { index: usize, total: usize },
    /// Данные видео из плейлиста стали известны во время загрузки (для строки прогресса).
    Meta { index: usize, data: VideoPreview },
    Plan { index: usize, subs: bool, audio_n: usize, copy: bool, audio_only: bool },
    Progress(Progress),
    StageDone { index: usize, stage: Stage, size: u64, elapsed: f64, merged: bool },
    TcStart { index: usize },
    /// Перекодирование не нужно — поток копируется как есть.
    TcMode { index: usize, copy: bool },
    ItemDone { index: usize, path: String, size: u64, srt: Vec<String> },
    ItemError { index: usize, total: usize, message: String, stage: Stage },
    ItemCancelled { index: usize, stage: Stage },
    Finished { saved: usize, failed: usize, total: usize, bytes: u64, elapsed: f64, cancelled: bool },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Stage { Download, Transcode }

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LogLevel {
    #[serde(rename = "n")] Normal,
    #[serde(rename = "c")] Command,
    #[serde(rename = "w")] Warn,
    #[serde(rename = "e")] Error,
    #[serde(rename = "ok")] Ok,
}

/// Прогресс; поля, которых у стадии нет, — None.
#[derive(Debug, Clone, Default, Serialize)]
pub struct Progress {
    pub index: usize,
    pub stage: Option<Stage>,
    /// download: "video" | "audio" | "subs" | "merge"
    pub step: Option<String>,
    pub pct: Option<f64>,
    pub done: Option<u64>,
    pub total: Option<u64>,
    /// байт/с при загрузке; множитель скорости (1.8 = 1.8x) при перекодировании
    pub speed: Option<f64>,
    pub eta: Option<f64>,
    pub audio_i: Option<usize>,
    pub audio_n: Option<usize>,
    pub frame: Option<u64>,
    pub frames: Option<u64>,
    /// уже переведённая подпись («Получение данных…»)
    pub note: Option<String>,
}

/// Карточка превью одного видео (preview_data в Python).
#[derive(Debug, Clone, Default, Serialize)]
pub struct VideoPreview {
    pub url: String,
    pub title: String,
    pub channel: String,
    pub duration: Option<f64>,
    pub views: Option<u64>,
    /// YYYYMMDD
    pub date: Option<String>,
    pub thumb_url: Option<String>,
    /// Байты обложки (jpeg/png/webp), уже скачанные движком с правильным Referer. None — нет обложки.
    #[serde(skip)]
    pub thumb: Option<Arc<Vec<u8>>>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    pub vcodec: Option<String>,
    pub heights: Vec<u32>,
    pub vfmts: Vec<VideoFormat>,
    pub tracks: Vec<AudioTrack>,
    /// коды языков официальных субтитров
    pub subs: Vec<String>,
    pub auto_subs: bool,
    pub language: Option<String>,
    pub audio_only: bool,
    pub site: String,
    pub youtube: bool,
    /// превью получено со входом через cookies из браузера (расширение)
    pub browser_login: bool,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct VideoFormat { pub h: u32, pub fps: Option<f64>, pub vc: String, pub hdr: bool, pub https: bool, pub tbr: Option<f64> }

#[derive(Debug, Clone, Default, Serialize)]
pub struct AudioTrack { pub lang: String, pub original: bool, pub auto: bool }

#[derive(Debug, Clone, Default, Serialize)]
pub struct PlaylistPreview {
    pub url: String,
    pub title: String,
    pub channel: String,
    pub site: String,
    pub youtube: bool,
    pub thumb_url: Option<String>,
    #[serde(skip)]
    pub thumb: Option<Arc<Vec<u8>>>,
    pub entries: Vec<PlaylistEntry>,
    pub duration: Option<f64>,
}

#[derive(Debug, Clone, Default, Serialize)]
pub struct PlaylistEntry { pub url: String, pub title: String, pub duration: Option<f64> }

#[derive(Debug, Clone)]
pub enum PreviewResult {
    Video(VideoPreview),
    Playlist(PlaylistPreview),
    /// error — понятный текст на языке интерфейса; raw — исходная строка ошибки yt-dlp.
    Error { error: String, raw: String },
}

/// Одна ссылка в очереди. Для видео из плейлиста folder = название плейлиста, prefix = «01 ».
#[derive(Debug, Clone, Default)]
pub struct Item { pub url: String, pub folder: String, pub prefix: String }

/// Настройки для конкретной ссылки (ключ — url).
#[derive(Debug, Clone, Default)]
pub struct Override {
    /// Правило озвучек для этой ссылки (orig | orig_ui | orig_ui_en | all): None — из настроек.
    pub audio: Option<String>,
    /// Фрагмент: (начало, конец) в секундах; конец None — до конца видео.
    pub trim: Option<(f64, Option<f64>)>,
}

#[derive(Debug, Clone)]
pub struct InitInfo {
    pub settings: Settings,
    /// какие из yt-dlp / ffmpeg / ffprobe не найдены
    pub missing: Vec<String>,
    /// итоговый язык интерфейса (auto уже раскрыт)
    pub lang: String,
    pub system_lang: String,
    pub portable: bool,
}

#[derive(Debug, Clone)]
pub struct GpuInfo { pub vendor: String, pub short: String }

/// Cookies, присланные расширением вместе со ссылкой. Только в памяти; ключ — site_key (домен без www).
#[derive(Default)]
pub struct BrowserCookies { inner: std::sync::Mutex<HashMap<String, String>> }

impl BrowserCookies {
    /// Сохранить cookies.txt (формат Netscape) для сайта.
    pub fn put(&self, site: String, netscape: String) { self.inner.lock().unwrap().insert(site, netscape); }
    /// Найти cookies для ссылки: тот же сайт, его поддомен или родительский домен.
    pub fn for_url(&self, url: &str) -> Option<String> {
        let key = site_key(url);
        let m = self.inner.lock().unwrap();
        m.iter().find(|(site, _)| key == **site || key.ends_with(&format!(".{site}")) || site.ends_with(&format!(".{key}")))
            .map(|(_, t)| t.clone())
    }
}

/// Домен ссылки без www./m., youtu.be -> youtube.com (как site_key в Python).
pub fn site_key(url: &str) -> String {
    let rest = url.split("://").nth(1).unwrap_or(url);
    let host = rest.split(['/', '?', '#']).next().unwrap_or("");
    let host = host.rsplit('@').next().unwrap_or(host);
    let host = host.split(':').next().unwrap_or(host).to_ascii_lowercase();
    let host = host.strip_prefix("www.").or_else(|| host.strip_prefix("m.")).unwrap_or(&host).to_string();
    if host == "youtu.be" { "youtube.com".into() } else { host }
}


/// Общее состояние движка: его видят очередь, превью и обновление yt-dlp.
pub(crate) struct Shared {
    pub sink: EventSink,
    pub cookies: Arc<BrowserCookies>,
    lang: Mutex<String>,
    pub info_cache: Mutex<HashMap<String, preview::CacheEntry>>,
    pub updater: updater::Updater,
    gpu_started: AtomicBool,
    /// None — ещё определяется; Some(None) — подходящей видеокарты нет.
    gpu: Mutex<Option<Option<encode::Gpu>>>,
    gpu_cv: Condvar,
}

impl Shared {
    pub fn lang(&self) -> String { self.lang.lock().unwrap().clone() }

    fn set_lang(&self, setting: &str) { *self.lang.lock().unwrap() = i18n::resolve_lang(setting); }

    /// Запустить определение видеокарты в фоне (один раз).
    fn start_gpu(self: &Arc<Self>) {
        if self.gpu_started.swap(true, Ordering::SeqCst) {
            return;
        }
        let me = self.clone();
        std::thread::spawn(move || {
            let g = encode::detect_gpu();
            *me.gpu.lock().unwrap() = Some(g);
            me.gpu_cv.notify_all();
        });
    }

    fn gpu_wait(self: &Arc<Self>, timeout: Duration) -> Option<encode::Gpu> {
        self.start_gpu();
        let g = self.gpu.lock().unwrap();
        let (g, _) = self.gpu_cv.wait_timeout_while(g, timeout, |g| g.is_none()).unwrap();
        g.clone().flatten()
    }

    /// Видеокарта для перекодирования (ждём определения до 30 с).
    pub fn gpu_for_job(self: &Arc<Self>) -> Option<encode::Gpu> { self.gpu_wait(Duration::from_secs(30)) }
}

struct Running {
    ctl: Arc<job::Ctl>,
    handle: std::thread::JoinHandle<()>,
}

/// Движок. Один на процесс; создаётся в app и живёт до закрытия окна.
pub struct Engine {
    sink: EventSink,
    pub cookies: Arc<BrowserCookies>,
    sh: Arc<Shared>,
    job: Mutex<Option<Running>>,
}

impl Engine {
    pub fn new(sink: EventSink) -> Arc<Engine> {
        let cookies: Arc<BrowserCookies> = Arc::default();
        let sh = Arc::new(Shared {
            sink: sink.clone(),
            cookies: cookies.clone(),
            lang: Mutex::new(i18n::resolve_lang(&Settings::load().lang)),
            info_cache: Mutex::default(),
            updater: updater::Updater::default(),
            gpu_started: AtomicBool::new(false),
            gpu: Mutex::new(None),
            gpu_cv: Condvar::new(),
        });
        Arc::new(Engine { sink, cookies, sh, job: Mutex::new(None) })
    }

    /// Настройки, отсутствующие инструменты, язык. Быстро, можно звать из UI-потока.
    pub fn init(&self) -> InitInfo {
        let missing = ["yt-dlp", "ffmpeg", "ffprobe"].iter().filter(|t| paths::tool(t).is_none()).map(|t| t.to_string()).collect();
        let settings = Settings::load();
        self.sh.set_lang(&settings.lang);
        InitInfo { settings, missing, lang: self.sh.lang(), system_lang: i18n::system_lang(), portable: paths::portable() }
    }

    pub fn save_settings(&self, s: &Settings) {
        s.save();
        self.sh.set_lang(&s.lang);
    }

    /// Блокирующий вызов (до ~3 мин): звать из фонового потока.
    pub fn preview(&self, url: &str) -> PreviewResult { preview::preview(&self.sh, url) }

    /// false — очередь уже идёт. Сохраняет settings, запускает очередь в фоне; события — в sink.
    pub fn start(&self, items: Vec<Item>, settings: Settings, overrides: HashMap<String, Override>) -> bool {
        let mut g = self.job.lock().unwrap();
        if g.as_ref().is_some_and(|r| !r.handle.is_finished()) {
            return false;
        }
        settings.save();
        let tasks: Vec<job::Task> = items
            .into_iter()
            .filter(|i| !i.url.is_empty())
            .enumerate()
            .map(|(i, it)| {
                let ov = overrides.get(&it.url);
                let trim = ov.and_then(|o| o.trim).map(|(a, b)| (if a.is_finite() { a } else { 0.0 }, b.filter(|x| x.is_finite())));
                let audio = ov.and_then(|o| o.audio.as_deref()).filter(|a| !a.is_empty()).map(media::AudioRule::parse);
                job::Task::new(i, it.url, util::safe_name(&it.folder, 120), it.prefix, trim, audio)
            })
            .collect();
        let ctl = Arc::new(job::Ctl::new(self.sink.clone()));
        let j = Arc::new(job::Job::new(self.sh.clone(), ctl.clone(), settings, tasks.len()));
        let handle = std::thread::Builder::new()
            .name("topicture-job".into())
            .spawn(move || j.run(tasks))
            .expect("spawn job thread");
        *g = Some(Running { ctl, handle });
        true
    }

    fn running(&self) -> Option<Arc<job::Ctl>> {
        self.job.lock().unwrap().as_ref().filter(|r| !r.handle.is_finished()).map(|r| r.ctl.clone())
    }

    pub fn pause(&self, on: bool) {
        if let Some(c) = self.running() {
            c.set_paused(on);
        }
    }

    pub fn cancel(&self) {
        if let Some(c) = self.running() {
            c.cancel();
        }
    }

    pub fn is_running(&self) -> bool { self.running().is_some() }

    /// Очередь на паузе.
    pub fn is_paused(&self) -> bool { self.running().is_some_and(|c| c.is_paused()) }

    /// PID запущенных сейчас процессов очереди (корни деревьев yt-dlp / ffmpeg).
    pub fn job_pids(&self) -> Vec<u32> { self.running().map(|c| c.pids()).unwrap_or_default() }

    /// Блокирует до окончания определения видеокарты (до 30 с) — звать из фона.
    pub fn gpu_info(&self) -> Option<GpuInfo> {
        self.sh.gpu_wait(Duration::from_secs(30)).map(|g| GpuInfo { vendor: g.vendor.into(), short: g.short.into() })
    }

    /// Строка версий для журнала; заодно запускает в фоне проверку обновления yt-dlp и определение GPU.
    pub fn versions(&self) -> String {
        let sh = self.sh.clone();
        std::thread::spawn(move || {
            sh.updater.check(&sh, false);
        });
        self.sh.start_gpu();
        format!("{APP} {APP_VERSION}{} · {}", if paths::portable() { " portable" } else { "" }, tool_versions())
    }

    /// Перевод на текущем языке интерфейса.
    pub fn tr(&self, key: &str, params: &[(&str, &str)]) -> String { i18n::tr(&self.sh.lang(), key, params) }

    /// Вызывать при закрытии окна: отменяет очередь.
    pub fn shutdown(&self) { self.cancel(); }
}

/// «yt-dlp 2025.01.01 · ffmpeg 7.1» для журнала.
pub fn tool_versions() -> String {
    let q = |a: &[&str]| procs::run_quiet(&a.iter().map(|s| s.to_string()).collect::<Vec<_>>(), Duration::from_secs(20));
    let mut parts = vec![];
    match q(&["yt-dlp", "--version"]) {
        Ok(o) => parts.push(format!("yt-dlp {}", String::from_utf8_lossy(&o.stdout).trim())),
        Err(_) => parts.push("yt-dlp —".into()),
    }
    let ff = q(&["ffmpeg", "-version"]).ok().and_then(|o| String::from_utf8_lossy(&o.stdout).lines().next().map(str::to_string));
    match ff {
        Some(first) => {
            let v = regex::Regex::new(r"version\s+(\S+)").unwrap().captures(&first).map(|c| c[1].chars().take(20).collect::<String>());
            parts.push(format!("ffmpeg {}", v.unwrap_or_else(|| "?".into())));
        }
        None => parts.push("ffmpeg —".into()),
    }
    parts.join(" · ")
}
