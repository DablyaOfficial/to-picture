//! To Picture — движок: настройки, переводы, превью (yt-dlp -J), очередь загрузок и перекодирования.
//! Порт логики из ToPicture.pyw (Python-версия 1.0.0). Интерфейса здесь нет: события уходят в `EventSink`.
//!
//! КОНТРАКТ. Публичные типы и сигнатуры в этом файле согласованы между crates (core / bridge / app).
//! Менять их можно только добавлением полей/методов; переименование и удаление ломают другие части.

pub mod paths;    // INSTALL_DIR, DATA_DIR (portable.txt, TOPICTURE_DATA), BIN_DIR, CONFIG, STATE
pub mod i18n;     // ui/locales/*.json, встроенные в exe через include_str!; tr(), множественные формы
pub mod settings; // Settings (совместим с settings.json Python-версии, schema 3), load/save, state.json

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;

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

/// Движок. Один на процесс; создаётся в app и живёт до закрытия окна.
pub struct Engine {
    sink: EventSink,
    pub cookies: Arc<BrowserCookies>,
    // остальное — на усмотрение реализации (очередь, кэш превью, GPU, обновление yt-dlp)
}

impl Engine {
    pub fn new(sink: EventSink) -> Arc<Engine> {
        let _ = &sink;
        Arc::new(Engine { sink, cookies: Arc::default() })
    }
    /// Настройки, отсутствующие инструменты, язык. Быстро, можно звать из UI-потока.
    pub fn init(&self) -> InitInfo { todo!("core: init") }
    pub fn save_settings(&self, s: &Settings) { let _ = s; todo!("core: save_settings") }
    /// Блокирующий вызов (до ~3 мин): звать из фонового потока.
    pub fn preview(&self, url: &str) -> PreviewResult { let _ = url; todo!("core: preview") }
    /// false — очередь уже идёт. Сохраняет settings, запускает очередь в фоне; события — в sink.
    pub fn start(&self, items: Vec<Item>, settings: Settings, overrides: HashMap<String, Override>) -> bool {
        let _ = (items, settings, overrides, &self.sink);
        todo!("core: start")
    }
    pub fn pause(&self, on: bool) { let _ = on; todo!("core: pause") }
    pub fn cancel(&self) { todo!("core: cancel") }
    pub fn is_running(&self) -> bool { todo!("core: is_running") }
    /// Блокирует до окончания определения видеокарты (до 30 с) — звать из фона.
    pub fn gpu_info(&self) -> Option<GpuInfo> { todo!("core: gpu_info") }
    /// Строка версий для журнала; заодно запускает в фоне проверку обновления yt-dlp и определение GPU.
    pub fn versions(&self) -> String { todo!("core: versions") }
    /// Перевод на текущем языке интерфейса.
    pub fn tr(&self, key: &str, params: &[(&str, &str)]) -> String { let _ = (key, params); todo!("core: tr") }
    /// Вызывать при закрытии окна: отменяет очередь.
    pub fn shutdown(&self) { todo!("core: shutdown") }
}
