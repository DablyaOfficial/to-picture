//! settings.json и state.json — формат и миграции как в Python-версии (обновление с 1.x сохраняет настройки).
use crate::paths;
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

pub const AUDIO_RULES: &[&str] = &["orig", "orig_ui", "orig_ui_en", "all"];
pub const SUBS_MODES: &[&str] = &["off", "embed", "embed_srt"];
pub const SUBS_RULES: &[&str] = &["ui_en", "ui", "en", "audio", "all"];
pub const VIDEO_CODECS: &[&str] = &["h264", "h265", "prores", "copy"];
pub const AUDIO_CODECS: &[&str] = &["mp3", "flac", "m4a"];
pub const NAME_TPLS: &[&str] = &["title", "channel_title", "date_title", "title_id"];
pub const RATE_LIMITS: &[&str] = &["off", "1M", "2M", "5M", "10M", "25M"];
pub const COOKIE_MODES: &[&str] = &["off", "firefox", "file"];
pub const SPONSOR_MODES: &[&str] = &["off", "remove", "mark"];

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Settings {
    pub out_dir: String,
    /// h264 | h265 | prores | copy | mp3 | flac | m4a
    pub codec: String,
    pub crf: i64,
    pub preset: String,
    /// "best" или высота строкой ("1080")
    pub max_height: String,
    pub audio: String,
    /// system | light | dark
    pub theme: String,
    /// auto или код языка
    pub lang: String,
    pub audio_lang: String,
    pub subs: String,
    pub subs_lang: String,
    /// gpu | cpu
    pub encoder: String,
    pub name_tpl: String,
    pub sponsorblock: String,
    /// on | off
    pub parallel: String,
    pub rate_limit: String,
    pub cookies: String,
    pub cookies_file: String,
    pub proxy: String,
    pub settings_open: bool,
    pub schema: i64,
}

impl Default for Settings {
    fn default() -> Self {
        Settings {
            out_dir: paths::home().join("Videos").join("To Picture").to_string_lossy().into_owned(),
            codec: "h264".into(),
            crf: 16,
            preset: "slow".into(),
            max_height: "best".into(),
            audio: "320k".into(),
            theme: "system".into(),
            lang: "auto".into(),
            audio_lang: "orig_ui".into(),
            subs: "embed".into(),
            subs_lang: "ui_en".into(),
            encoder: "gpu".into(),
            name_tpl: "title".into(),
            sponsorblock: "off".into(),
            parallel: "on".into(),
            rate_limit: "off".into(),
            cookies: "off".into(),
            cookies_file: String::new(),
            proxy: String::new(),
            settings_open: false,
            schema: 3,
        }
    }
}

impl Settings {
    pub fn is_audio_out(&self) -> bool { AUDIO_CODECS.contains(&self.codec.as_str()) }

    /// Загрузить с миграциями. Ошибки чтения -> значения по умолчанию.
    pub fn load() -> Settings {
        let mut data = serde_json::to_value(Settings::default()).unwrap();
        let obj = data.as_object_mut().unwrap();
        let mut loaded = false;
        if let Some(raw) = read_json(&paths::config_file()) {
            merge(obj, &raw);
            loaded = true;
        }
        if !loaded {
            if let Some(raw) = paths::old_config_file().and_then(|p| read_json(&p)) {
                let mut m = Map::new();
                for k in ["out_dir", "crf", "preset"] {
                    if let Some(v) = raw.get(k) { m.insert(k.into(), v.clone()); }
                }
                merge(obj, &m);
            }
        }
        // max_height мог быть числом в старых версиях
        if let Some(v) = obj.get("max_height").and_then(Value::as_i64) {
            obj.insert("max_height".into(), Value::String(v.to_string()));
        }
        let mut s: Settings = serde_json::from_value(data).unwrap_or_default();
        s.migrate();
        s
    }

    fn migrate(&mut self) {
        if self.schema < 2 {
            self.audio_lang = "orig_ui".into();
            self.subs = "embed".into();
            self.schema = 2;
        }
        if self.schema < 3 {
            self.audio_lang = match self.audio_lang.as_str() { "orig_ru" => "orig_ui", "orig_ru_en" => "orig_ui_en", o => o }.into();
            self.subs_lang = match self.subs_lang.as_str() { "ru_en" => "ui_en", "ru" => "ui", o => o }.into();
            self.schema = 3;
        }
        if self.subs == "file" { self.subs = "embed_srt".into(); }
        fix(&mut self.audio_lang, AUDIO_RULES, "orig_ui");
        fix(&mut self.subs_lang, SUBS_RULES, "ui_en");
        fix(&mut self.subs, SUBS_MODES, "embed");
        if !VIDEO_CODECS.contains(&self.codec.as_str()) && !AUDIO_CODECS.contains(&self.codec.as_str()) {
            self.codec = "h264".into();
        }
        fix(&mut self.name_tpl, NAME_TPLS, "title");
        fix(&mut self.sponsorblock, SPONSOR_MODES, "off");
        fix(&mut self.parallel, &["on", "off"], "on");
        fix(&mut self.rate_limit, RATE_LIMITS, "off");
        fix(&mut self.cookies, COOKIE_MODES, "off");
        fix(&mut self.encoder, &["gpu", "cpu"], "gpu");
        fix(&mut self.theme, &["system", "light", "dark"], "system");
        self.crf = self.crf.clamp(10, 30);
    }

    pub fn save(&self) {
        let p = paths::config_file();
        if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
        if let Ok(text) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(p, text);
        }
    }
}

fn fix(v: &mut String, allowed: &[&str], default: &str) {
    if !allowed.contains(&v.as_str()) { *v = default.into(); }
}

fn merge(dst: &mut Map<String, Value>, src: &Map<String, Value>) {
    for (k, v) in src {
        if dst.contains_key(k) && !v.is_null() {
            // тип должен совпадать, иначе оставляем значение по умолчанию
            let same = std::mem::discriminant(&dst[k]) == std::mem::discriminant(v)
                || (k == "max_height" && v.is_number());
            if same { dst.insert(k.clone(), v.clone()); }
        }
    }
}

fn read_json(p: &std::path::Path) -> Option<Map<String, Value>> {
    let text = std::fs::read_to_string(p).ok()?;
    serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')).ok()?.as_object().cloned()
}

/// state.json: служебные отметки (последняя проверка yt-dlp, ext_seen, ext_help_shown …).
pub fn load_state() -> Map<String, Value> {
    read_json(&paths::state_file()).unwrap_or_default()
}

pub fn save_state(data: &Map<String, Value>) {
    let p = paths::state_file();
    if let Some(d) = p.parent() { let _ = std::fs::create_dir_all(d); }
    if let Ok(text) = serde_json::to_string_pretty(data) { let _ = std::fs::write(p, text); }
}
