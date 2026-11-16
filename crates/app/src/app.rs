//! Логика окна — перенос скрипта из ui/index.html: список ссылок, превью, параметры, очередь, журнал,
//! меню и окно расширения. Состояние живёт в потоке интерфейса (Rc<RefCell<App>>), движок шлёт события
//! из своих потоков через slint::invoke_from_event_loop.
use crate::backend::Backend;
use crate::fmt::{self, base, t};
use crate::{icu, win};
use crate::{AppWindow, Measure, Cell, GLine, LogLine, MItem, Opt, PRow, Pv, SelData, Step, SubItem, ULine, S, T};
use regex::Regex;
use slint::{ComponentHandle, Image, Model, ModelRc, Rgba8Pixel, SharedPixelBuffer, SharedString, Timer, VecModel};
use std::cell::RefCell;
use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;
use std::sync::{Arc, LazyLock};
use std::time::Duration;
use topicture_bridge::InboxLink;
use topicture_core::settings::AUDIO_CODECS;
use topicture_core::*;

thread_local! {
    static APP: RefCell<Option<Rc<RefCell<App>>>> = const { RefCell::new(None) };
}

/// Выполнить f с состоянием окна (в потоке интерфейса). Если состояние занято (вызов изнутри другого
/// обработчика) — отложить на следующий оборот цикла событий.
pub fn with_app(f: impl FnOnce(&mut App) + 'static) {
    let Some(rc) = APP.with(|a| a.borrow().clone()) else { return };
    match rc.try_borrow_mut() {
        Ok(mut a) => f(&mut a),
        Err(_) => Timer::single_shot(Duration::ZERO, move || with_app(f)),
    }
}

/// Из любого потока: выполнить f в потоке интерфейса.
pub fn post(f: impl FnOnce(&mut App) + Send + 'static) {
    let _ = slint::invoke_from_event_loop(move || with_app(f));
}

static URL_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)^https?://\S+$").unwrap());
static PLAYLIST_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)[?&]list=|/playlist\b|/@[^/?#]+/?(videos|shorts|streams|featured)?/?([?#]|$)|/(channel|c|user)/|/sets/|/albums?/|/playlists?/").unwrap()
});
static NOT_PLAYLIST_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"(?i)[?&]v=|/watch\b|/shorts/[\w-]{6,}").unwrap());

fn looks_like_playlist(u: &str) -> bool {
    PLAYLIST_RE.is_match(u) && !NOT_PLAYLIST_RE.is_match(u)
}

const NB: &str = "\u{202F}"; // узкий неразрывный пробел перед %

fn ss(s: impl AsRef<str>) -> SharedString { SharedString::from(s.as_ref()) }
fn model<T: Clone + 'static>(v: Vec<T>) -> ModelRc<T> { ModelRc::new(VecModel::from(v)) }

// ---------------------------------------------------------------- данные

/// Результат превью (ok — видео или плейлист).
#[derive(Clone)]
pub enum PvData {
    Video(Arc<VideoPreview>),
    Playlist(Arc<PlaylistPreview>),
    Error(String),
}

impl PvData {
    fn ok(&self) -> bool { !matches!(self, PvData::Error(_)) }
    fn title(&self) -> Option<&str> {
        match self {
            PvData::Video(v) => Some(&v.title),
            PvData::Playlist(p) => Some(&p.title),
            PvData::Error(_) => None,
        }
    }
}

/// Озвучка для одной ссылки: правило или явный список языков.
#[derive(Clone, PartialEq)]
enum AudioOv { Rule(String), Langs(Vec<String>) }

#[derive(Clone, Copy, PartialEq)]
enum Ls { Pending, Active, Done, Error }

#[derive(Clone, Copy, PartialEq, Default)]
enum Rs { #[default] Pending, Active, Done, Error }

#[derive(Clone, Default)]
struct RowSt {
    label: String,
    idx: Option<usize>,
    state: Rs,
    pct: f64,
    step: Option<String>,
    l: String,
    r: String,
    audio_n: usize,
    audio_i: usize,
    has_subs: bool,
    audio_only: bool,
    copy: bool,
}

#[derive(Clone, Default)]
struct RunItem { url: String, folder: String, prefix: String, title: String }

#[derive(Clone)]
struct O { v: String, l: String, h: String, mh: String, sep: bool }
fn o(v: &str, l: impl Into<String>) -> O { O { v: v.into(), l: l.into(), h: String::new(), mh: String::new(), sep: false } }
fn oh(v: &str, l: impl Into<String>, h: impl Into<String>, mh: impl Into<String>) -> O { O { v: v.into(), l: l.into(), h: h.into(), mh: mh.into(), sep: false } }
fn sep() -> O { O { v: String::new(), l: String::new(), h: String::new(), mh: String::new(), sep: true } }

/// Выбранный видеопоток (как select_video в Python).
#[derive(Clone)]
struct Vf { h: u32, fps: f64, vc: String, hdr: bool, https: bool, tbr: f64 }

pub struct App {
    ui: slint::Weak<AppWindow>,
    be: Arc<dyn Backend>,
    lang: String,
    sys_lang: String,
    langs: Vec<i18n::Lang>,
    portable: bool,
    s: Settings,
    def_subs_lang: &'static str,
    gpu: Option<GpuInfo>,
    missing: Vec<String>,
    versions: String,
    dark: Option<bool>,

    // ссылки
    text: String,
    list_mode: bool,
    run_urls: Vec<String>,
    line_state: Vec<Ls>,
    run_labels: Vec<String>,
    run_items: Vec<RunItem>,
    src_urls: Vec<String>,

    // очередь
    running: bool,
    cancelling: bool,
    preparing: bool,
    prep_urls: Vec<String>,
    paused: bool,
    locked: bool,
    cur: isize,
    total: usize,
    dl: RowSt,
    tc: RowSt,
    rows_shown: bool,
    settings_at_run: Settings,
    plans: HashMap<usize, (bool, bool)>,
    last_file: Option<String>,
    last_size: u64,
    last_srt: Vec<String>,

    // превью
    done: HashMap<String, PvData>,
    pending: HashSet<String>,
    thumbs: HashMap<String, Image>,
    shown_url: Option<String>,
    shown: Option<(String, PvData)>,
    pv_gen: u64,
    pv_timer: Timer,
    resolved: HashMap<String, PvData>,
    audio_ov: HashMap<String, AudioOv>,
    trim: HashMap<String, (f64, Option<f64>)>,
    pl_sel: HashMap<String, BTreeSet<usize>>,

    // журнал и подсказки
    log: Rc<VecModel<LogLine>>,
    log_open: bool,
    log_last: String,
    hint_flash: String,
    hint_timer: Timer,
    copy_timer: Timer,
    save_timer: Timer,
    theme_timer: Timer,
    widths: HashMap<String, f32>,
    ext_folder: String,
}

impl App {
    pub fn install(ui: &AppWindow, be: Arc<dyn Backend>) -> Rc<RefCell<App>> {
        let init = be.init();
        let mut app = App {
            ui: ui.as_weak(),
            be,
            lang: init.lang.clone(),
            sys_lang: init.system_lang.clone(),
            langs: i18n::available_langs(),
            portable: init.portable,
            s: init.settings.clone(),
            def_subs_lang: "ui_en",
            gpu: None,
            missing: init.missing.clone(),
            versions: String::new(),
            dark: None,
            text: String::new(),
            list_mode: false,
            run_urls: vec![],
            line_state: vec![],
            run_labels: vec![],
            run_items: vec![],
            src_urls: vec![],
            running: false,
            cancelling: false,
            preparing: false,
            prep_urls: vec![],
            paused: false,
            locked: false,
            cur: -1,
            total: 0,
            dl: RowSt::default(),
            tc: RowSt::default(),
            rows_shown: false,
            settings_at_run: init.settings.clone(),
            plans: HashMap::new(),
            last_file: None,
            last_size: 0,
            last_srt: vec![],
            done: HashMap::new(),
            pending: HashSet::new(),
            thumbs: HashMap::new(),
            shown_url: None,
            shown: None,
            pv_gen: 0,
            pv_timer: Timer::default(),
            resolved: HashMap::new(),
            audio_ov: HashMap::new(),
            trim: HashMap::new(),
            pl_sel: HashMap::new(),
            log: Rc::new(VecModel::default()),
            log_open: false,
            log_last: String::new(),
            hint_flash: String::new(),
            hint_timer: Timer::default(),
            copy_timer: Timer::default(),
            save_timer: Timer::default(),
            theme_timer: Timer::default(),
            widths: HashMap::new(),
            ext_folder: String::new(),
        };
        app.normalize_for_lang();
        let rc = Rc::new(RefCell::new(app));
        APP.with(|a| *a.borrow_mut() = Some(rc.clone()));
        wire(ui);
        {
            let mut a = rc.borrow_mut();
            let s = ui.global::<S>();
            s.set_lines(ModelRc::from(a.log.clone()));
            a.apply_lang_fonts();
            a.render_all();
            a.render_preview_state_empty();
            // тема системы может смениться, пока окно открыто
            a.theme_timer.start(slint::TimerMode::Repeated, Duration::from_secs(2), || with_app(|a| a.apply_theme()));
        }
        rc
    }

    fn ui(&self) -> AppWindow { self.ui.upgrade().expect("window") }

    fn tr(&self, key: &str) -> String { t(&self.lang, key, &[]) }
    fn trp(&self, key: &str, p: &[(&str, &str)]) -> String { t(&self.lang, key, p) }

    /// языки, где названия языков пишутся со строчной (_lower)
    fn lc(&self, x: &str) -> String {
        if i18n::meta(&self.lang, "_lower").and_then(|v| v.as_bool()).unwrap_or(false) { x.to_lowercase() } else { x.to_string() }
    }

    fn lang_name(&self, code: &str) -> String {
        if code.is_empty() || code == "und" {
            return self.tr("lang.none");
        }
        let n = icu::language_name(&self.lang, code).unwrap_or_else(|| code.to_string());
        let mut c = n.chars();
        match c.next() {
            Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
            None => n,
        }
    }
    fn t_name(&self, code: &str) -> String { self.lang_name(&base(code)) }
    fn ui_lang_name(&self) -> String { self.lang_name(&base(&self.lang)) }
    fn lang_native(&self, code: &str) -> String {
        self.langs.iter().find(|l| l.code == code).map(|l| l.name.clone()).unwrap_or_else(|| code.to_string())
    }

    fn normalize_for_lang(&mut self) {
        let en = base(&self.lang) == "en";
        self.def_subs_lang = if en { "en" } else { "ui_en" };
        if en {
            if self.s.audio_lang == "orig_ui_en" { self.s.audio_lang = "orig_ui".into(); }
            if self.s.subs_lang == "ui_en" || self.s.subs_lang == "ui" { self.s.subs_lang = "en".into(); }
        }
    }

    fn apply_lang_fonts(&self) {
        let ui = self.ui();
        let fonts = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into()).join("Fonts");
        let var = fonts.join("SegUIVar.ttf").exists();
        let sans = match base(&self.lang).as_str() {
            "zh" => "Microsoft YaHei UI",
            "ja" => "Yu Gothic UI",
            "ko" => "Malgun Gothic",
            _ if var => "Segoe UI Variable Text",
            _ => "Segoe UI",
        };
        let mono = if fonts.join("CascadiaMono.ttf").exists() { "Cascadia Mono" } else { "Consolas" };
        let th = ui.global::<T>();
        th.set_sans(sans.into());
        th.set_mono(mono.into());
        ui.global::<S>().set_lang(self.lang.clone().into());
    }

    // ------------------------------------------------------------ варианты

    fn opts(&self, key: &str) -> Vec<O> {
        let lang = self.ui_lang_name();
        let lang_lc = self.lc(&lang);
        let p = [("lang", lang.as_str()), ("lang_lc", lang_lc.as_str())];
        let ui_en = base(&self.lang) == "en";
        match key {
            "codec" => vec![
                oh("h264", "H.264", self.tr("codec.h264.h"), self.tr("codec.h264.mh")),
                oh("h265", "H.265", self.tr("codec.h265.h"), self.tr("codec.h265.mh")),
                oh("prores", "ProRes 422", self.tr("codec.prores.h"), self.tr("codec.prores.mh")),
                sep(),
                oh("copy", self.tr("codec.copy.l"), self.tr("codec.copy.h"), self.tr("codec.copy.mh")),
                sep(),
                oh("mp3", "MP3", self.tr("codec.audio.h"), self.tr("codec.mp3.mh")),
                oh("flac", "FLAC", self.tr("codec.audio.h"), self.tr("codec.flac.mh")),
                oh("m4a", "M4A", self.tr("codec.audio.h"), self.tr("codec.m4a.mh")),
            ],
            "max_height" => {
                let mut v = vec![o("best", self.tr("res.best"))];
                v.extend(["2160", "1440", "1080", "720", "480"].iter().map(|h| o(h, format!("{h}p"))));
                v
            }
            "preset" => ["ultrafast", "superfast", "veryfast", "faster", "fast", "medium", "slow", "slower", "veryslow"]
                .iter()
                .map(|p| {
                    let h = if matches!(*p, "ultrafast" | "medium" | "slow" | "veryslow") { self.tr(&format!("preset.{p}")) } else { String::new() };
                    oh(p, *p, h, "")
                })
                .collect(),
            "audio" => {
                let k = if self.s.codec == "mp3" { "audio.kbpsMp3" } else { "audio.kbps" };
                ["128", "192", "256", "320"].iter().map(|n| o(&format!("{n}k"), self.trp(k, &[("n", n)]))).collect()
            }
            "name_tpl" => ["title", "channel_title", "date_title", "title_id"].iter().map(|v| o(v, self.tr(&format!("name.{v}")))).collect(),
            "parallel" => vec![oh("on", self.tr("par.on"), "", self.tr("par.on.mh")), oh("off", self.tr("par.off"), "", self.tr("par.off.mh"))],
            "sponsorblock" => vec![
                o("off", self.tr("sb.off")),
                oh("remove", self.tr("sb.remove"), "", self.tr("sb.remove.mh")),
                oh("mark", self.tr("sb.mark"), "", self.tr("sb.mark.mh")),
            ],
            "cookies" => vec![
                o("off", self.tr("ck.off")),
                oh("firefox", "Firefox", "", self.tr("ck.firefox.mh")),
                oh("file", self.tr("ck.file"), "", self.tr("ck.file.mh")),
            ],
            "rate_limit" => {
                let mut v = vec![o("off", self.tr("rate.off"))];
                v.extend(["1", "2", "5", "10", "25"].iter().map(|n| o(&format!("{n}M"), self.trp("rate.n", &[("n", n)]))));
                v
            }
            "theme" => vec![o("system", self.tr("theme.system")), o("light", self.tr("theme.light")), o("dark", self.tr("theme.dark"))],
            "audio_lang" => {
                let mut v = vec![o("orig", self.tr("alang.orig"))];
                if ui_en {
                    v.push(oh("orig_ui", self.tr("alang.orig_en"), self.tr("alang.orig_ui.h"), self.trp("alang.orig_ui.mh", &p)));
                } else {
                    v.push(oh("orig_ui", self.trp("alang.orig_ui", &p), self.tr("alang.orig_ui.h"), self.trp("alang.orig_ui.mh", &p)));
                    v.push(oh("orig_ui_en", self.trp("alang.orig_ui_en", &p), "", self.tr("alang.orig_ui_en.mh")));
                }
                v.push(sep());
                v.push(oh("all", self.tr("alang.all"), "", self.tr("alang.all.mh")));
                v
            }
            "subs" => vec![
                o("off", self.tr("subs.off")),
                oh("embed", self.tr("subs.embed"), self.tr("subs.embed.h"), self.tr("subs.embed.mh")),
                oh("embed_srt", self.tr("subs.embed_srt"), self.tr("subs.embed_srt.h"), self.tr("subs.embed_srt.mh")),
            ],
            "subs_lang" => {
                let mut v = vec![];
                if !ui_en {
                    v.push(o("ui_en", self.trp("slang.ui_en", &p)));
                    v.push(o("ui", self.trp("slang.ui", &p)));
                }
                v.push(o("en", self.tr("slang.en")));
                v.push(oh("audio", self.tr("slang.audio"), "", self.tr("slang.audio.mh")));
                v.push(o("all", self.tr("slang.all")));
                v
            }
            "encoder" => vec![
                oh("gpu", self.tr("enc.gpu"), self.gpu.as_ref().map(|g| g.vendor.clone()).unwrap_or_default(), self.tr("enc.gpu.mh")),
                oh("cpu", self.tr("enc.cpu"), "", self.tr("enc.cpu.mh")),
            ],
            _ => vec![],
        }
    }

    fn opt_of(&self, key: &str, v: &str) -> O {
        let list = self.opts(key);
        list.iter().find(|x| !x.sep && x.v == v).or_else(|| list.iter().find(|x| !x.sep)).cloned().unwrap_or_else(|| o(v, v))
    }

    fn get(&self, key: &str) -> String {
        let s = &self.s;
        match key {
            "codec" => s.codec.clone(),
            "max_height" => s.max_height.clone(),
            "preset" => s.preset.clone(),
            "audio" => s.audio.clone(),
            "name_tpl" => s.name_tpl.clone(),
            "parallel" => s.parallel.clone(),
            "sponsorblock" => s.sponsorblock.clone(),
            "cookies" => s.cookies.clone(),
            "rate_limit" => s.rate_limit.clone(),
            "theme" => s.theme.clone(),
            "audio_lang" => s.audio_lang.clone(),
            "subs" => s.subs.clone(),
            "subs_lang" => s.subs_lang.clone(),
            "encoder" => s.encoder.clone(),
            _ => String::new(),
        }
    }

    fn set(&mut self, key: &str, v: String) {
        let s = &mut self.s;
        match key {
            "codec" => s.codec = v,
            "max_height" => s.max_height = v,
            "preset" => s.preset = v,
            "audio" => s.audio = v,
            "name_tpl" => s.name_tpl = v,
            "parallel" => s.parallel = v,
            "sponsorblock" => s.sponsorblock = v,
            "cookies" => s.cookies = v,
            "rate_limit" => s.rate_limit = v,
            "theme" => s.theme = v,
            "audio_lang" => s.audio_lang = v,
            "subs" => s.subs = v,
            "subs_lang" => s.subs_lang = v,
            "encoder" => s.encoder = v,
            _ => {}
        }
    }

    fn sel_data(&self, key: &str, dis: bool) -> SelData {
        let opts = self.opts(key);
        let cur = self.get(key);
        let idx = opts.iter().position(|x| !x.sep && x.v == cur).or_else(|| opts.iter().position(|x| !x.sep)).unwrap_or(0);
        let c = opts.get(idx).cloned().unwrap_or_else(|| o("", ""));
        SelData {
            label: ss(&c.l),
            hint: ss(&c.h),
            tip: ss(if c.mh.is_empty() { String::new() } else { format!("{} — {}", c.l, c.mh) }),
            value: ss(&cur),
            dis,
            idx: idx as i32,
            nseps: opts.iter().filter(|x| x.sep).count() as i32,
            opts: model(opts.into_iter().map(|x| Opt { v: ss(x.v), l: ss(x.l), h: ss(x.h), mh: ss(x.mh), sep: x.sep }).collect()),
        }
    }

    fn is_audio_out(&self) -> bool { AUDIO_CODECS.contains(&self.s.codec.as_str()) }
    fn use_gpu(&self) -> bool { self.gpu.is_some() && self.s.encoder == "gpu" && (self.s.codec == "h264" || self.s.codec == "h265") }
    fn crf_word(&self, n: i64) -> String {
        self.tr(if n <= 13 { "crf.1" } else if n <= 17 { "crf.2" } else if n <= 21 { "crf.3" } else if n <= 25 { "crf.4" } else { "crf.5" })
    }

    // ------------------------------------------------------------ параметры

    fn settings_summary(&self) -> String {
        let s = &self.s;
        let c = self.opt_of("codec", &s.codec);
        let res = self.lc(&self.opt_of("max_height", &s.max_height).l);
        let mut parts: Vec<String> = vec![];
        if s.codec == "h264" || s.codec == "h265" {
            parts.push(c.l.clone() + &if self.use_gpu() { format!(" · {}", self.gpu.as_ref().unwrap().short) } else { String::new() });
            parts.push(format!("CRF {}", s.crf));
            parts.push(s.preset.clone());
            parts.push(self.opt_of("audio", &s.audio).l);
        } else if s.codec == "prores" {
            parts.push("ProRes 422".into());
            parts.push("PCM".into());
        } else if s.codec == "flac" {
            parts.push(self.trp("sum.audioOnly", &[("f", "FLAC")]));
        } else if self.is_audio_out() {
            parts.push(self.trp("sum.audioOnly", &[("f", &s.codec.to_uppercase())]));
            parts.push(self.opt_of("audio", &s.audio).l);
        } else {
            parts.push(self.tr("sum.noReencode"));
        }
        if !self.is_audio_out() { parts.push(res); }
        if s.audio_lang != "orig_ui" {
            parts.push(self.trp("sum.audio", &[("v", &self.lc(&self.opt_of("audio_lang", &s.audio_lang).l))]));
        }
        if s.subs == "off" {
            parts.push(self.tr("sum.noSubs"));
        } else if s.subs != "embed" || s.subs_lang != self.def_subs_lang {
            let v = self.lc(&self.opt_of("subs_lang", &s.subs_lang).l) + if s.subs == "embed_srt" { " + .srt" } else { "" };
            parts.push(self.trp("sum.subs", &[("v", &v)]));
        }
        if s.sponsorblock != "off" { parts.push(self.tr(&format!("sum.sb.{}", s.sponsorblock))); }
        if s.cookies != "off" {
            parts.push(self.trp("sum.cookies", &[("v", if s.cookies == "firefox" { "Firefox" } else { "cookies.txt" })]));
        }
        if !s.proxy.trim().is_empty() { parts.push(self.tr("sum.proxy")); }
        if s.rate_limit != "off" {
            parts.push(self.trp("sum.rate", &[("n", s.rate_limit.trim_end_matches('M'))]));
        }
        parts.push(s.out_dir.clone());
        parts.join(" · ")
    }

    fn codec_desc(&self, s: &Settings) -> String {
        if s.codec == "h264" || s.codec == "h265" {
            let gpu = if self.use_gpu() { self.tr("desc.onGpu") } else { String::new() };
            return self.trp("desc.codec", &[("codec", &self.opt_of("codec", &s.codec).l), ("gpu", &gpu), ("crf", &s.crf.to_string()), ("preset", &s.preset)]);
        }
        if s.codec == "prores" { return "ProRes 422, PCM".into(); }
        if s.codec == "flac" { return "FLAC".into(); }
        if AUDIO_CODECS.contains(&s.codec.as_str()) {
            return self.opt_of("audio", &s.audio).l.replace("AAC", if s.codec == "m4a" { "M4A AAC" } else { "AAC" });
        }
        self.tr("desc.noReencode")
    }

    fn fit(&self, id: &str, full: &str, px: f32, mono: bool) -> String {
        match self.widths.get(id) {
            Some(w) => fmt::fit_middle(full, *w, px, mono),
            None => full.to_string(),
        }
    }

    fn render_settings(&mut self) {
        let ui = self.ui();
        let s = ui.global::<S>();
        let st = self.s.clone();
        let lock = self.locked;
        let a_out = self.is_audio_out();
        s.set_sel_codec(self.sel_data("codec", lock));
        s.set_sel_preset(self.sel_data("preset", lock));
        s.set_sel_audio(self.sel_data("audio", lock));
        s.set_sel_encoder(self.sel_data("encoder", lock));
        s.set_sel_audio_lang(self.sel_data("audio_lang", lock));
        s.set_sel_subs(self.sel_data("subs", lock));
        s.set_sel_subs_lang(self.sel_data("subs_lang", lock));
        s.set_sel_name_tpl(self.sel_data("name_tpl", lock));
        s.set_sel_parallel(self.sel_data("parallel", lock));
        s.set_sel_sponsorblock(self.sel_data("sponsorblock", lock));
        s.set_sel_cookies(self.sel_data("cookies", lock));
        s.set_sel_rate_limit(self.sel_data("rate_limit", lock));
        s.set_sel_max_height(self.sel_data("max_height", lock || a_out));
        s.set_crf(st.crf as i32);
        s.set_crf_word(self.crf_word(st.crf).into());
        s.set_enc_hint(
            if st.encoder == "gpu" {
                let g = self.gpu.as_ref().map(|g| format!("{} {}", g.vendor, g.short)).unwrap_or_default();
                self.trp("enc.hintGpu", &[("gpu", &g)])
            } else {
                self.tr("enc.hintCpu")
            }
            .into(),
        );
        s.set_ck_hint(self.tr(&format!("ck.hint.{}", st.cookies)).into());
        s.set_cookies_file_empty(st.cookies_file.is_empty());
        if st.cookies_file.is_empty() {
            s.set_cookies_file(self.tr("ck.noFile").into());
            s.set_cookies_file_tip("".into());
        } else {
            s.set_cookies_file(self.fit("cookiesFile", &st.cookies_file, 13.0, false).into());
            s.set_cookies_file_tip(st.cookies_file.clone().into());
        }
        if s.get_proxy() != st.proxy.as_str() && !self.widths.contains_key("proxyFocus") {
            s.set_proxy(st.proxy.clone().into());
        }
        s.set_out_dir(self.fit("outDir", &st.out_dir, 13.0, false).into());
        s.set_out_dir_tip(st.out_dir.clone().into());
        s.set_set_sum(self.settings_summary().into());
        s.set_set_open(st.settings_open);
        s.set_locked(lock);
        self.render_grid();
        if let Some((_, d)) = self.shown.clone() {
            self.render_preview(&d);
        }
        self.apply_theme();
    }

    /// Раскладка сетки параметров — как авторазмещение CSS Grid в index.html (4 колонки, в узком окне 2).
    fn render_grid(&self) {
        #[derive(Clone, Copy)]
        enum K { A, W, H, F }
        let st = &self.s;
        let x264 = st.codec == "h264" || st.codec == "h265";
        let gpu_on = x264 && self.gpu.is_some();
        let abr = x264 || st.codec == "mp3" || st.codec == "m4a";
        let vid = !self.is_audio_out();
        let subs_on = vid && st.subs != "off";
        let subs_off = vid && st.subs == "off";
        let ck_file = st.cookies == "file";
        use K::*;
        let spec: [(K, bool); 40] = [
            (A, true), (W, true),
            (A, true), (A, true), (A, true), (A, true),
            (A, x264), (W, x264),
            (A, x264), (A, x264), (A, abr), (A, abr),
            (A, gpu_on), (A, gpu_on), (H, gpu_on),
            (A, true), (A, true), (H, true),
            (A, vid), (A, vid), (A, subs_on), (A, subs_on), (H, subs_off),
            (A, true), (A, true), (A, true), (A, true),
            (A, true), (A, true), (H, true),
            (F, true),
            (A, true), (A, true), (H, true),
            (A, ck_file), (W, ck_file),
            (A, true), (A, true), (A, true), (A, true),
        ];
        let width = self.widths.get("win").copied().unwrap_or(820.0);
        let n: i32 = if width < 780.0 { 2 } else { 4 };
        let (mut r, mut c) = (0i32, 0i32);
        let cells: Vec<Cell> = spec
            .iter()
            .map(|&(k, show)| {
                if !show {
                    return Cell { show: false, row: 0, col: 0, span: 1 };
                }
                let (start, span) = match k {
                    A => {
                        if c + 1 > n { r += 1; c = 0; }
                        let p = c;
                        c += 1;
                        return Cell { show: true, row: r, col: p, span: 1 };
                    }
                    W => (1, n - 1),
                    H => if n == 4 { (2, 2) } else { (1, 1) },
                    F => (0, n),
                };
                if start < c { r += 1; }
                c = start + span;
                Cell { show: true, row: r, col: start, span }
            })
            .collect();
        let lab = |i: usize| -> &str { match i { 0 => "set.folder", 2 => "set.codec", 4 => "set.resolution", 6 => "set.quality", 8 => "set.speed", 10 => "set.sound", 12 => "set.encoder", 15 => "set.audio", 18 => "set.subs", 20 => "set.subsLang", 23 => "set.name", 25 => "set.queue", 27 => "set.sponsor", 31 => "set.cookies", 36 => "set.proxy", 38 => "set.rate", _ => "" } };
        let (mut l0, mut l2) = (88f32, 88f32);
        for (i, c) in cells.iter().enumerate() {
            let k = lab(i);
            if !c.show || k.is_empty() { continue; }
            let w = fmt::text_width(&self.tr(k), 13.0, false).ceil() + 3.0;
            if c.col == 0 { l0 = l0.max(w); } else if c.col == 2 { l2 = l2.max(w); }
        }
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_cells(model(cells));
        s.set_grid_l0(l0);
        s.set_grid_l2(l2);
        s.set_narrow_grid(n == 2);
    }

    fn on_settings_changed(&mut self) {
        self.render_settings();
        self.save_timer.start(slint::TimerMode::SingleShot, Duration::from_millis(300), || {
            with_app(|a| a.be.save_settings(&a.s));
        });
    }

    pub fn apply_theme(&mut self) {
        let dark = self.s.theme == "dark" || (self.s.theme == "system" && win::system_dark());
        if self.dark != Some(dark) {
            self.dark = Some(dark);
            let ui = self.ui();
            ui.global::<T>().set_dark(dark);
            win::set_dark_title(ui.window(), dark);
        }
    }

    fn lock_settings(&mut self, lock: bool) {
        self.locked = lock;
        self.render_settings();
    }

    /// вход и прокси меняют то, что сайт отдаёт, — превью с ошибкой пробуем заново
    fn on_net_changed(&mut self) {
        self.done.retain(|_, d| d.ok());
        if self.shown.is_none() {
            self.shown_url = None;
            self.schedule_preview();
        }
    }

    // ------------------------------------------------------------ список ссылок

    fn text_lines(&self) -> Vec<&str> { self.text.split('\n').collect() }
    fn valid_urls(&self) -> Vec<String> {
        self.text.split('\n').map(str::trim).filter(|s| URL_RE.is_match(s)).map(String::from).collect()
    }

    fn render_gutter(&self) {
        let lines: Vec<GLine> = if self.list_mode {
            self.run_urls.iter().enumerate().map(|(i, _)| GLine {
                n: i as i32 + 1,
                mark: match self.line_state.get(i) { Some(Ls::Active) => 1, Some(Ls::Done) => 2, Some(Ls::Error) => 3, _ => 0 },
            }).collect()
        } else {
            (1..=self.text_lines().len().max(1)).map(|n| GLine { n: n as i32, mark: 0 }).collect()
        };
        self.ui().global::<S>().set_gutter(model(lines));
    }

    fn render_url_list(&self) {
        let v: Vec<ULine> = self.run_urls.iter().enumerate().map(|(i, u)| ULine {
            text: ss(self.run_labels.get(i).filter(|l| !l.is_empty()).unwrap_or(u)),
            done: self.line_state.get(i) == Some(&Ls::Done),
            tip: ss(u),
        }).collect();
        self.ui().global::<S>().set_url_list(model(v));
    }

    fn set_list_mode(&mut self, on: bool) {
        self.list_mode = on;
        self.ui().global::<S>().set_list_mode(on);
        if on { self.render_url_list(); }
        self.render_gutter();
        self.update_buttons();
    }

    fn edit_list(&mut self) {
        if self.running || !self.list_mode { return; }
        self.set_list_mode(false);
    }

    fn set_text(&mut self, text: String) {
        self.text = text.clone();
        self.ui().global::<S>().set_urls(text.into());
    }

    fn append_text(&mut self, text: &str) {
        let text = text.trim();
        if text.is_empty() { return; }
        if self.list_mode { self.set_list_mode(false); }
        let cur = self.text.trim_end().to_string();
        let new = if cur.is_empty() { format!("{text}\n") } else { format!("{cur}\n{text}\n") };
        let n = new.len();
        self.set_text(new);
        self.on_list_changed();
        let ui = self.ui();
        ui.invoke_urls_to_end(n as i32);
        ui.invoke_focus_urls();
    }

    fn on_list_changed(&mut self) {
        let live: HashSet<String> = self.valid_urls().into_iter().collect();
        self.audio_ov.retain(|u, _| live.contains(u));
        self.trim.retain(|u, _| live.contains(u));
        self.pl_sel.retain(|u, _| live.contains(u));
        self.render_gutter();
        self.update_buttons();
        self.schedule_preview();
    }

    // ------------------------------------------------------------ превью

    fn last_valid_url(&self) -> Option<String> { self.valid_urls().pop() }

    fn schedule_preview(&mut self) {
        self.pv_timer.start(slint::TimerMode::SingleShot, Duration::from_millis(400), || {
            with_app(|a| {
                if !a.running {
                    let u = a.last_valid_url();
                    a.show_preview_for(u, true);
                }
            })
        });
    }

    /// Запросить превью в фоне (если ещё не запрошено). Ответ придёт в preview_done.
    fn get_preview(&mut self, url: &str) {
        if self.done.contains_key(url) || self.pending.contains(url) {
            return;
        }
        self.pending.insert(url.to_string());
        let be = self.be.clone();
        let url = url.to_string();
        std::thread::spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| be.preview(&url))).unwrap_or(PreviewResult::Error { error: String::new(), raw: "panic".into() });
            // обложку декодируем здесь же, чтобы не задерживать окно
            let bytes = match &r {
                PreviewResult::Video(v) => v.thumb.clone(),
                PreviewResult::Playlist(p) => p.thumb.clone(),
                _ => None,
            };
            let px = bytes.and_then(|b| decode_thumb(&b));
            post(move |a| a.preview_done(url, r, px));
        });
    }

    fn preview_done(&mut self, url: String, r: PreviewResult, px: Option<SharedPixelBuffer<Rgba8Pixel>>) {
        self.pending.remove(&url);
        if let Some(px) = px {
            self.thumbs.insert(url.clone(), Image::from_rgba8(px));
        }
        let d = match r {
            PreviewResult::Video(v) => PvData::Video(Arc::new(v)),
            PreviewResult::Playlist(p) => PvData::Playlist(Arc::new(p)),
            PreviewResult::Error { error, .. } => {
                PvData::Error(if error.is_empty() { self.tr("pv.fetchFailed") } else { error })
            }
        };
        self.done.insert(url.clone(), d.clone());
        if self.running && self.run_items.iter().any(|i| i.url == url && i.folder.is_empty()) {
            self.resolved.insert(url.clone(), d.clone());
            if self.cur >= 0 && (self.cur as usize) + 1 < self.total {
                self.render_queue_label();
            }
        }
        if self.preparing && self.prep_urls.contains(&url) {
            self.try_start();
        }
        if self.shown_url.as_deref() == Some(url.as_str()) && self.shown.is_none() {
            self.show_loaded(&url);
        }
    }

    fn show_preview_for(&mut self, url: Option<String>, from_edit: bool) {
        if url == self.shown_url && self.shown.is_some() {
            return;
        }
        self.shown_url = url.clone();
        self.pv_gen += 1;
        let Some(url) = url else {
            self.shown = None;
            let has_text = !self.text.trim().is_empty() && from_edit;
            if has_text {
                let last = self.text_lines().into_iter().rfind(|s| !s.trim().is_empty()).unwrap_or("").to_string();
                self.render_preview_state_error(&self.tr("pv.unrecognized"), &self.tr("pv.badLink"), &last);
            } else {
                self.render_preview_state_empty();
            }
            return;
        };
        self.shown = None;
        if self.done.contains_key(&url) {
            self.show_loaded(&url);
        } else {
            self.render_preview_state_loading(&url);
            self.get_preview(&url);
        }
    }

    fn show_loaded(&mut self, url: &str) {
        let Some(d) = self.done.get(url).cloned() else { return };
        if !d.ok() {
            // ошибку не кешируем — можно попробовать ещё раз
            self.done.remove(url);
            self.shown = None;
            let PvData::Error(e) = d else { unreachable!() };
            self.render_preview_state_error(&self.tr("pv.unavailable"), &e, url);
            return;
        }
        self.shown = Some((url.to_string(), d.clone()));
        self.render_preview(&d);
    }

    fn pv_base(&self) -> Pv {
        Pv { kw: 72.0, ..Default::default() }
    }

    fn render_preview_state_empty(&self) {
        let pv = Pv { state: 0, msg: ss(self.tr("pv.noLink")), ..self.pv_base() };
        self.ui().global::<S>().set_pv(pv);
    }
    fn render_preview_state_loading(&self, url: &str) {
        let pv = Pv { state: 1, url: ss(url), ..self.pv_base() };
        self.ui().global::<S>().set_pv(pv);
    }
    fn render_preview_state_error(&self, title: &str, err: &str, url: &str) {
        let pv = Pv { state: 2, title: ss(title), err: ss(err), url: ss(url), ..self.pv_base() };
        self.ui().global::<S>().set_pv(pv);
    }

    fn select_video(&self, d: &VideoPreview) -> (Option<Vf>, bool) {
        let v: Vec<Vf> = d.vfmts.iter().map(|f| Vf { h: f.h, fps: f.fps.unwrap_or(0.0), vc: f.vc.clone(), hdr: f.hdr, https: f.https, tbr: f.tbr.unwrap_or(0.0) }).collect();
        if v.is_empty() {
            return (
                d.height.filter(|h| *h > 0).map(|h| Vf { h, fps: d.fps.unwrap_or(0.0).round(), vc: d.vcodec.clone().unwrap_or_default(), hdr: false, https: true, tbr: 0.0 }),
                false,
            );
        }
        let s = &self.s;
        let lim: u32 = if s.max_height == "best" { 0 } else { s.max_height.parse().unwrap_or(0) };
        let trimmed = !d.url.is_empty() && self.trim.contains_key(&d.url);
        let fit: Vec<Vf> = v.iter().filter(|f| lim == 0 || f.h <= lim).cloned().collect();
        let pool = if fit.is_empty() { v } else { fit };
        let better = |a: &Vf, b: &Vf| -> f64 {
            let c = |x: f64| if x != 0.0 { x } else { 0.0 };
            let d1 = a.h as f64 - b.h as f64;
            if d1 != 0.0 { return d1; }
            let d2 = a.fps - b.fps;
            if d2 != 0.0 { return d2; }
            let d3 = a.https as i32 as f64 - b.https as i32 as f64;
            if d3 != 0.0 { return d3; }
            c(a.tbr - b.tbr)
        };
        let best = |list: &[Vf]| -> Vf { list.iter().skip(1).fold(list[0].clone(), |a, b| if better(&a, b) >= 0.0 { a } else { b.clone() }) };
        if s.codec == "copy" {
            return (Some(best(&pool)), false);
        }
        let sdr_only: Vec<Vf> = pool.iter().filter(|f| !f.hdr).cloned().collect();
        let sdr = if sdr_only.is_empty() { pool } else { sdr_only };
        let b = best(&sdr);
        if s.codec == "h264" && !trimmed {
            let avc: Vec<Vf> = sdr.iter().filter(|f| f.vc.starts_with("avc1") && f.h == b.h && f.fps == b.fps).cloned().collect();
            if !avc.is_empty() {
                return (Some(best(&avc)), true);
            }
        }
        (Some(b), false)
    }

    fn output_label(&self, d: Option<&VideoPreview>) -> String {
        let s = &self.s;
        if s.codec == "flac" { return "FLAC".into(); }
        if self.is_audio_out() {
            let l = self.opt_of("audio", &s.audio).l;
            let rest = l.split_once(char::is_whitespace).map(|x| x.1.to_string()).unwrap_or(l);
            return format!("{} · {}", s.codec.to_uppercase(), rest);
        }
        let empty = VideoPreview::default();
        let d = d.unwrap_or(&empty);
        if d.audio_only { return "M4A".into(); }
        let (f, copy) = self.select_video(d);
        let q = f.as_ref().map(|f| fmt::q_tag(f.h, Some(f.fps))).unwrap_or_default();
        let c = if copy {
            self.tr("pv.noReencode")
        } else if s.codec == "copy" {
            f.as_ref().map(|f| f.vc.clone()).filter(|v| !v.is_empty()).unwrap_or_else(|| self.tr("pv.source"))
        } else {
            match s.codec.as_str() { "h264" => "H.264".into(), "h265" => "H.265".into(), "prores" => "ProRes 422".into(), x => x.into() }
        };
        let bx = if !self.subs_chosen(d).is_empty() || s.codec == "copy" { "MKV" } else if s.codec == "prores" { "MOV" } else { "MP4" };
        [if copy { String::new() } else { q }, c, bx.to_string()].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")
    }

    fn thumb_for(&self, url: &str) -> Option<Image> { self.thumbs.get(url).cloned() }

    fn render_preview(&mut self, d: &PvData) {
        match d {
            PvData::Video(v) => self.render_video(v),
            PvData::Playlist(p) => self.render_playlist(p),
            PvData::Error(_) => {}
        }
    }

    fn key_width(&self, keys: &[&str]) -> f32 {
        keys.iter().map(|k| fmt::text_width(&self.tr(k), 12.0, false).ceil() + 1.0).fold(72.0, f32::max)
    }

    fn render_video(&mut self, d: &VideoPreview) {
        let url = self.shown.as_ref().map(|x| x.0.clone()).unwrap_or_default();
        let mut d = d.clone();
        d.url = url.clone();
        let site = if !d.youtube { d.site.clone() } else { String::new() };
        let chan = if !d.channel.is_empty() && !site.is_empty() && site.to_lowercase().starts_with(&d.channel.to_lowercase()) { String::new() } else { d.channel.clone() };
        let byline = [site, chan, fmt::views(&self.lang, d.views), fmt::date(&self.lang, d.date.as_deref())]
            .into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
        let sv = if d.audio_only { None } else { self.select_video(&d).0 };
        let src = if d.audio_only {
            self.tr("pv.audioOnly")
        } else if let Some(f) = &sv {
            [fmt::q_tag(f.h, Some(f.fps)), f.vc.clone() + if f.hdr { " HDR" } else { "" }].into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ")
        } else {
            String::new()
        };
        let tr = self.trim.get(&url).copied();
        let trim = match tr {
            Some((a, b)) => {
                let end = match b { Some(b) => fmt::time(b), None => { let e = fmt::time_opt(d.duration); if e.is_empty() { "…".into() } else { e } } };
                format!("{}–{}", fmt::time(a), end)
            }
            None => self.tr("pv.trim"),
        };
        let mut pv = Pv {
            state: 3,
            has_thumb: self.thumbs.contains_key(&url),
            thumb: self.thumb_for(&url).unwrap_or_default(),
            msg: ss(self.tr("pv.noThumb")),
            chip: ss(fmt::time_opt(d.duration.filter(|x| *x > 0.0))),
            title: ss(&d.title),
            byline: ss(byline),
            src: ss(src),
            out: ss(self.output_label(Some(&d))),
            trim: ss(trim),
            trim_on: tr.is_some(),
            kw: self.key_width(&["pv.video", "pv.audio", "pv.subs"]),
            ..Default::default()
        };
        // ---- озвучка
        let (tracks, ov, rule, cur) = self.current_audio(&d);
        if tracks.len() <= 1 {
            let known = !tracks.is_empty() && tracks[0].lang != "und";
            pv.akind = if known { 1 } else { 0 };
            if known { pv.atext = ss(self.t_name(&tracks[0].lang)); }
        } else {
            let n = tracks.len();
            pv.akind = 2;
            pv.alabel = ss(if cur.len() == n { self.trp("pv.all", &[("n", &n.to_string())]) } else { self.names_short(&cur.iter().map(|x| x.lang.clone()).collect::<Vec<_>>(), 2) });
            pv.anote = ss(if self.is_audio_out() && cur.len() > 1 {
                self.trp("pv.oneTrack", &[("name", &self.lc(&self.t_name(&cur[0].lang)))])
            } else if ov.is_some() {
                self.trp("pv.noteOverride", &[("names", &self.lc(&self.names_short(&rule.iter().map(|x| x.lang.clone()).collect::<Vec<_>>(), 3)))])
            } else {
                self.trp("pv.noteRule", &[("n", &n.to_string())])
            });
            pv.aov = ov.is_some();
            let list = cur.iter().map(|x| self.t_name(&x.lang) + &if x.original { self.tr("pv.originalParen") } else { String::new() }).collect::<Vec<_>>().join(", ");
            pv.atip = ss(self.trp("pv.embedTip", &[("list", &list)]));
        }
        // ---- субтитры
        let avail = &d.subs;
        if self.is_audio_out() {
            pv.skind = 0;
            pv.sa = ss(self.tr("pv.subsAudio"));
        } else if avail.is_empty() {
            if d.auto_subs { pv.skind = 1; pv.sa = ss(self.tr("pv.subsAutoOnly")); } else { pv.skind = 0; pv.sa = ss(self.tr("pv.none")); }
        } else {
            let cnt = self.trp("pv.langs", &[("n", &avail.len().to_string())]);
            let chosen = self.subs_chosen(&d);
            if self.s.subs == "off" {
                pv.skind = 2;
                pv.sbtn = ss(&cnt);
                pv.sa = ss(self.tr("pv.notDownloaded"));
            } else if !chosen.is_empty() {
                pv.skind = 3;
                pv.sa = ss(self.names_short(&chosen, 3));
                pv.stip = ss(chosen.iter().map(|c| self.t_name(c)).collect::<Vec<_>>().join(", "));
                pv.sb = ss(self.tr(if self.s.subs == "embed_srt" { "pv.inVideoSrt" } else { "pv.inVideo" }));
                pv.sbtn = ss(&cnt);
            } else {
                pv.skind = 4;
                pv.sa = ss(self.tr("pv.noWanted"));
                pv.sbtn = ss(self.trp("pv.have", &[("cnt", &cnt)]));
            }
        }
        self.ui().global::<S>().set_pv(pv);
    }

    fn pl_selected(&self, d: &PlaylistPreview, url: &str) -> BTreeSet<usize> {
        self.pl_sel.get(url).cloned().unwrap_or_else(|| (0..d.entries.len()).collect())
    }

    fn render_playlist(&mut self, d: &PlaylistPreview) {
        let url = self.shown.as_ref().map(|x| x.0.clone()).unwrap_or_default();
        let count = d.entries.len();
        let byline = [d.site.clone(), if !d.channel.is_empty() && d.channel != d.title { d.channel.clone() } else { String::new() }, fmt::time_opt(d.duration.filter(|x| *x > 0.0))]
            .into_iter().filter(|x| !x.is_empty()).collect::<Vec<_>>().join(" · ");
        let n = self.pl_selected(d, &url).len();
        let label = if n == count { self.trp("pl.all", &[("n", &n.to_string())]) } else { self.trp("pl.some", &[("a", &n.to_string()), ("b", &count.to_string())]) };
        let audio = if self.is_audio_out() { "—".to_string() } else { self.lc(&self.opt_of("audio_lang", &self.s.audio_lang).l) };
        let subs = if self.is_audio_out() || self.s.subs == "off" { self.tr("pv.none") } else { self.lc(&self.opt_of("subs_lang", &self.s.subs_lang).l) };
        let pv = Pv {
            state: 4,
            has_thumb: self.thumbs.contains_key(&url),
            thumb: self.thumb_for(&url).unwrap_or_default(),
            msg: ss(self.tr("pv.noThumb")),
            chip: ss(self.trp("pl.count", &[("n", &count.to_string())])),
            title: ss(&d.title),
            byline: ss(byline),
            pl_label: ss(label),
            pl_folder: ss(self.trp("pl.folder", &[("name", &d.title)])),
            pl_files: ss(self.output_label(None)),
            pl_audio: ss(audio),
            pl_note: ss(format!("· {}: {}", self.tr("pv.subs").to_lowercase(), subs)),
            kw: self.key_width(&["pv.video", "pl.files", "pv.audio"]),
            ..Default::default()
        };
        self.ui().global::<S>().set_pv(pv);
        self.render_pl_items(d, &url);
    }

    fn render_pl_items(&self, d: &PlaylistPreview, url: &str) {
        let sel = self.pl_selected(d, url);
        let count = d.entries.len();
        let pad = count.to_string().len();
        let head = vec![
            MItem { kind: 0, id: ss("all"), label: ss(self.trp("pl.menuAll", &[("n", &count.to_string())])), sel: sel.len() == count, ..Default::default() },
            MItem { kind: 2, ..Default::default() },
        ];
        let items: Vec<MItem> = d.entries.iter().enumerate().map(|(i, e)| {
            let title = if e.title.is_empty() { e.url.clone() } else { e.title.clone() };
            MItem {
                kind: 3,
                id: ss(format!("i:{i}")),
                num: ss(format!("{:0pad$}", i + 1)),
                label: ss(&title),
                tip: ss(&title),
                hint: ss(fmt::time_opt(e.duration.filter(|x| *x > 0.0))),
                sel: sel.contains(&i),
                ..Default::default()
            }
        }).collect();
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_pl_head(model(head));
        s.set_pl_items(model(items));
    }

    fn pl_click(&mut self, id: &str) {
        let Some((url, PvData::Playlist(d))) = self.shown.clone() else { return };
        let count = d.entries.len();
        let mut sel = self.pl_selected(&d, &url);
        if id == "all" {
            sel = if sel.len() == count { [0].into() } else { (0..count).collect() };
        } else if let Some(i) = id.strip_prefix("i:").and_then(|x| x.parse::<usize>().ok()) {
            if sel.contains(&i) {
                if sel.len() == 1 { return; }
                sel.remove(&i);
            } else {
                sel.insert(i);
            }
        }
        if sel.len() == count { self.pl_sel.remove(&url); } else { self.pl_sel.insert(url.clone(), sel); }
        self.render_playlist(&d);
    }

    // ---- дорожки (pick_audio / pick_subs из Python)
    fn pick_audio(&self, tracks: &[AudioTrack], rule: &AudioOv) -> Vec<AudioTrack> {
        if tracks.len() <= 1 { return tracks.to_vec(); }
        let orig = tracks.iter().find(|t| t.original).unwrap_or(&tracks[0]).clone();
        match rule {
            AudioOv::Rule(r) if r == "all" => tracks.to_vec(),
            AudioOv::Langs(list) => {
                let want: Vec<String> = list.iter().map(|l| base(l)).collect();
                let pos = |t: &AudioTrack| want.iter().position(|w| *w == base(&t.lang)).unwrap_or(usize::MAX);
                let mut out: Vec<AudioTrack> = tracks.iter().filter(|t| want.contains(&base(&t.lang))).cloned().collect();
                out.sort_by(|a, b| if a.original == b.original { pos(a).cmp(&pos(b)) } else if a.original { std::cmp::Ordering::Less } else { std::cmp::Ordering::Greater });
                if out.is_empty() { vec![orig] } else { out }
            }
            AudioOv::Rule(r) => {
                let ui = base(&self.lang);
                let extra: Vec<String> = match r.as_str() { "orig" => vec![], "orig_ui_en" => vec![ui, "en".into()], _ => vec![ui] };
                let mut out = vec![orig];
                for l in extra {
                    if let Some(t) = tracks.iter().find(|t| base(&t.lang) == l)
                        && !out.iter().any(|o| o.lang == t.lang && o.original == t.original && o.auto == t.auto)
                    {
                        out.push(t.clone());
                    }
                }
                out
            }
        }
    }

    fn pick_subs(&self, avail: &[String], rule: &str, audio_langs: &[String], orig_lang: Option<&str>) -> Vec<String> {
        if rule == "all" { return avail.to_vec(); }
        let one = |l: &str| -> Vec<String> {
            let mut c: Vec<&String> = avail.iter().filter(|k| base(k) == l).collect();
            c.sort_by(|a, b| ((a.as_str() != l) as u8).cmp(&((b.as_str() != l) as u8)).then(a.cmp(b)));
            c.into_iter().take(1).cloned().collect()
        };
        let ui = base(&self.lang);
        let langs: Vec<String> = if rule == "audio" {
            if audio_langs.is_empty() { vec![base(orig_lang.unwrap_or(""))] } else { audio_langs.iter().map(|l| base(l)).collect() }
        } else {
            match rule { "ui" => vec![ui], "en" => vec!["en".into()], _ => vec![ui, "en".into()] }
        };
        let mut seen = HashSet::new();
        let mut out = vec![];
        for l in langs {
            if !l.is_empty() && seen.insert(l.clone()) {
                out.extend(one(&l));
            }
        }
        out
    }

    fn names_short(&self, codes: &[String], max: usize) -> String {
        let n: Vec<String> = codes.iter().map(|c| self.t_name(c)).collect();
        if n.len() <= max { n.join(", ") } else { format!("{} +{}", n[..max].join(", "), n.len() - max) }
    }

    /// (дорожки, выбор для ссылки, по правилу, текущие)
    fn current_audio(&self, d: &VideoPreview) -> (Vec<AudioTrack>, Option<AudioOv>, Vec<AudioTrack>, Vec<AudioTrack>) {
        let tracks = d.tracks.clone();
        let ov = self.audio_ov.get(&d.url).cloned();
        let rule_ov = AudioOv::Rule(self.s.audio_lang.clone());
        let rule = self.pick_audio(&tracks, &rule_ov);
        let cur = self.pick_audio(&tracks, ov.as_ref().unwrap_or(&rule_ov));
        (tracks, ov, rule, cur)
    }

    fn subs_chosen(&self, d: &VideoPreview) -> Vec<String> {
        if self.s.subs == "off" || self.is_audio_out() { return vec![]; }
        let (_, _, _, cur) = self.current_audio(d);
        self.pick_subs(&d.subs, &self.s.subs_lang, &cur.iter().map(|t| t.lang.clone()).collect::<Vec<_>>(), d.language.as_deref())
    }

    fn shown_video(&self) -> Option<VideoPreview> {
        match &self.shown {
            Some((u, PvData::Video(v))) => {
                let mut v = (**v).clone();
                v.url = u.clone();
                Some(v)
            }
            _ => None,
        }
    }

    fn render_track_menu(&self) {
        let Some(d) = self.shown_video() else { return };
        let (tracks, ov, rule, cur) = self.current_audio(&d);
        let n = tracks.len();
        let langs: Vec<String> = cur.iter().map(|t| t.lang.clone()).collect();
        let head = vec![
            MItem { kind: 0, id: ss("rule"), label: ss(self.tr("tm.byRule")), hint: ss(self.names_short(&rule.iter().map(|x| x.lang.clone()).collect::<Vec<_>>(), 3)), sel: ov.is_none(), ..Default::default() },
            MItem { kind: 2, ..Default::default() },
        ];
        let mut items = vec![];
        let groups: [(String, Vec<&AudioTrack>); 3] = [
            (self.tr("tm.original"), tracks.iter().filter(|x| x.original).collect()),
            (self.tr("tm.dub"), tracks.iter().filter(|x| !x.original && !x.auto).collect()),
            (self.tr("tm.autodub"), tracks.iter().filter(|x| !x.original && x.auto).collect()),
        ];
        for (g, mut list) in groups {
            if list.is_empty() { continue; }
            list.sort_by_key(|t| self.t_name(&t.lang).to_lowercase());
            items.push(MItem { kind: 1, label: ss(g), hint: ss(if list.len() > 1 { list.len().to_string() } else { String::new() }), ..Default::default() });
            for x in list {
                items.push(MItem { kind: 0, id: ss(format!("lang:{}", x.lang)), label: ss(self.t_name(&x.lang)), sel: langs.contains(&x.lang), ..Default::default() });
            }
        }
        let tail = vec![
            MItem { kind: 2, ..Default::default() },
            MItem { kind: 0, id: ss("all"), label: ss(self.trp("tm.all", &[("n", &n.to_string())])), hint: ss(self.tr("tm.allHint")), sel: cur.len() == n, ..Default::default() },
        ];
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_track_head(model(head));
        s.set_track_items(model(items));
        s.set_track_tail(model(tail));
    }

    /// true — закрыть меню
    fn track_click(&mut self, id: &str) -> bool {
        let Some(d) = self.shown_video() else { return true };
        let (tracks, _, rule, cur) = self.current_audio(&d);
        let n = tracks.len();
        let close = match id {
            "rule" => { self.audio_ov.remove(&d.url); true }
            "all" => { self.audio_ov.insert(d.url.clone(), AudioOv::Rule("all".into())); true }
            _ => {
                let Some(l) = id.strip_prefix("lang:") else { return false };
                let mut langs: Vec<String> = cur.iter().map(|t| t.lang.clone()).collect();
                if let Some(i) = langs.iter().position(|x| x == l) {
                    if langs.len() == 1 { return false; }
                    langs.remove(i);
                } else {
                    langs.push(l.to_string());
                }
                let rl: Vec<String> = rule.iter().map(|t| t.lang.clone()).collect();
                let same = langs.len() == rl.len() && langs.iter().all(|x| rl.contains(x));
                if same { self.audio_ov.remove(&d.url); }
                else if langs.len() == n { self.audio_ov.insert(d.url.clone(), AudioOv::Rule("all".into())); }
                else { self.audio_ov.insert(d.url.clone(), AudioOv::Langs(langs)); }
                false
            }
        };
        self.render_video(&d);
        self.render_track_menu();
        close
    }

    fn render_subs_pop(&self) {
        let Some(d) = self.shown_video() else { return };
        let avail = &d.subs;
        let chosen = self.subs_chosen(&d);
        let rule = self.opt_of("subs_lang", &self.s.subs_lang).l;
        let foot = if self.s.subs == "off" {
            self.tr("sp.off")
        } else if !chosen.is_empty() {
            let list = chosen.iter().map(|c| self.t_name(c)).collect::<Vec<_>>().join(", ");
            let srt = if self.s.subs == "embed_srt" { self.tr("sp.willSrt") } else { String::new() };
            self.trp("sp.will", &[("list", &list), ("srt", &srt), ("rule", &rule)])
        } else {
            self.trp("sp.missing", &[("rule", &rule)])
        };
        let mut sorted = avail.clone();
        sorted.sort_by_key(|k| self.t_name(k).to_lowercase());
        let items: Vec<SubItem> = sorted.iter().map(|k| SubItem { name: ss(self.t_name(k)), on: chosen.contains(k), tip: ss(k) }).collect();
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_subs_title(self.trp("sp.title", &[("n", &avail.len().to_string())]).into());
        s.set_subs_foot(foot.into());
        s.set_subs_items(model(items));
    }

    fn trim_open(&self) {
        let Some(d) = self.shown_video() else { return };
        let tr = self.trim.get(&d.url);
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_tr_start(tr.map(|t| fmt::time(t.0)).unwrap_or_else(|| "0:00".into()).into());
        s.set_tr_end(tr.and_then(|t| t.1).map(fmt::time).unwrap_or_default().into());
        s.set_tr_ph(d.duration.map(fmt::time).unwrap_or_else(|| self.tr("tr.endPh")).into());
        s.set_tr_msg(self.tr(if self.s.codec == "copy" { "tr.noteCopy" } else { "tr.note" }).into());
        s.set_tr_err(false);
        s.set_tr_has(tr.is_some());
    }

    fn trim_apply(&mut self, a: &str, b: &str) -> bool {
        let Some(d) = self.shown_video() else { return true };
        let a = fmt::parse_time(a).unwrap_or(0.0);
        let b = fmt::parse_time(b);
        let dur = d.duration.filter(|x| *x > 0.0);
        let bad = a.is_nan() || b.is_some_and(|b| b.is_nan() || b <= a) || dur.is_some_and(|du| a >= du);
        if bad {
            let ui = self.ui();
        let s = ui.global::<S>();
            s.set_tr_msg(self.tr("tr.bad").into());
            s.set_tr_err(true);
            return false;
        }
        let end = match (b, dur) { (Some(b), Some(du)) if b >= du => None, _ => b };
        if a == 0.0 && end.is_none() { self.trim.remove(&d.url); } else { self.trim.insert(d.url.clone(), (a, end)); }
        self.render_video(&d);
        true
    }

    // ------------------------------------------------------------ меню приложения

    fn render_app_menu(&self) {
        let v = format!("{}{}", APP_VERSION, if self.portable { " portable" } else { "" });
        let cur_lang = if self.s.lang == "auto" { self.trp("menu.langAuto", &[("name", &self.lang_native(&self.sys_lang))]) } else { self.lang_native(&self.s.lang) };
        let mut items = vec![MItem { kind: 1, label: ss(self.tr("menu.theme")), ..Default::default() }];
        for x in self.opts("theme") {
            items.push(MItem { kind: 0, id: ss(format!("theme:{}", x.v)), label: ss(&x.l), sel: self.s.theme == x.v, ..Default::default() });
        }
        items.push(MItem { kind: 2, ..Default::default() });
        items.push(MItem { kind: 0, id: ss("lang"), label: ss(self.tr("menu.lang")), hint: ss(format!("{cur_lang} ›")), dis: self.running, ..Default::default() });
        items.push(MItem { kind: 2, ..Default::default() });
        items.push(MItem { kind: 0, id: ss("ext"), label: ss(self.tr("menu.ext")), ..Default::default() });
        items.push(MItem { kind: 0, id: ss("data"), label: ss(self.tr("menu.data")), ..Default::default() });
        items.push(MItem { kind: 0, id: ss("about"), label: ss(self.tr("menu.about")), hint: ss(&v), ..Default::default() });
        let mut langs = vec![MItem { kind: 1, label: ss(self.tr("menu.lang")), ..Default::default() }];
        langs.push(MItem { kind: 0, id: ss("auto"), label: ss(self.trp("menu.langAuto", &[("name", &self.lang_native(&self.sys_lang))])), sel: self.s.lang == "auto", ..Default::default() });
        for l in &self.langs {
            let font = match base(&l.code).as_str() { "zh" => "Microsoft YaHei UI", "ja" => "Yu Gothic UI", "ko" => "Malgun Gothic", _ => "" };
            langs.push(MItem { kind: 0, id: ss(&l.code), label: ss(&l.name), sel: self.s.lang == l.code, font: ss(font), ..Default::default() });
        }
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_app_items(model(items));
        s.set_lang_items(model(langs));
    }

    fn app_act(&mut self, id: &str) -> &'static str {
        if let Some(th) = id.strip_prefix("theme:") {
            self.s.theme = th.into();
            self.on_settings_changed();
            self.render_app_menu();
            return "close";
        }
        match id {
            "lang" => return if self.running { "keep" } else { "lang" },
            "data" => {
                let d = paths::data_dir();
                let _ = std::fs::create_dir_all(&d);
                win::open(&d.to_string_lossy());
            }
            "ext" => self.open_ext(),
            "about" => {
                let v = format!("{}{}", APP_VERSION, if self.portable { " portable" } else { "" });
                self.set_log_open(true);
                self.append_log(&format!("To Picture {v} · {}", self.versions), 1);
                self.ui().invoke_reveal_log();
            }
            _ => {}
        }
        "close"
    }

    fn lang_act(&mut self, code: &str) {
        if code == self.s.lang || self.running { return; }
        self.s.lang = code.into();
        self.lang = i18n::resolve_lang(code);
        self.normalize_for_lang();
        self.be.save_settings(&self.s);
        self.apply_lang_fonts();
        self.render_all();
        // строки прогресса и превью пересобираются на новом языке
        if let Some((_, d)) = self.shown.clone() { self.render_preview(&d); }
        else if let Some(u) = self.shown_url.clone() { self.render_preview_state_loading(&u); }
        else { self.render_preview_state_empty(); }
    }

    // ------------------------------------------------------------ расширение

    fn open_ext(&mut self) {
        self.ui().invoke_close_popups();
        self.render_ext();
        self.ui().global::<S>().set_ext_show(true);
    }

    fn render_ext(&mut self) {
        let st = self.be.ext_status();
        self.ext_folder = st.folder.clone();
        let seen_ts = settings::load_state().get("ext_seen").and_then(|v| v.as_f64());
        let seen = match seen_ts { Some(ts) => self.trp("ext.seen", &[("t", &fmt::datetime(&self.lang, ts))]), None => self.tr("ext.notSeen") };
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_ext_ok(st.registered);
        if st.registered {
            s.set_ext_b(self.tr("ext.linked").into());
            s.set_ext_span(seen.into());
        } else {
            s.set_ext_b(self.tr("ext.notLinked").into());
            s.set_ext_span(self.tr("ext.notLinkedHint").into());
        }
        s.set_ext_connect_show(!st.registered);
        s.set_ext_path(self.fit("extPath", &st.folder, 12.0, true).into());
        s.set_ext_path_tip(st.folder.into());
    }

    fn ext_copy(&mut self, id: &str) {
        let text = match id { "chrome" => "chrome://extensions".to_string(), "edge" => "edge://extensions".to_string(), _ => self.ext_folder.clone() };
        set_clipboard(&text);
        self.ui().global::<S>().set_copied(id.into());
        self.copy_timer.start(slint::TimerMode::SingleShot, Duration::from_millis(1200), || {
            with_app(|a| a.ui().global::<S>().set_copied("".into()))
        });
    }

    /// Ссылки, присланные расширением (inbox). focus — поднять окно.
    pub fn take_inbox(&mut self, focus: bool) {
        let got = self.be.take_inbox();
        if got.is_empty() { return; }
        for m in &got {
            if let Some(c) = &m.cookies {
                self.be.put_cookies(site_key(&m.url), c.clone());
            }
        }
        for m in got {
            self.ext_link(m);
        }
        if focus {
            win::focus_or_flash(self.ui().window());
        }
    }

    fn ext_link(&mut self, m: InboxLink) {
        let url = m.url;
        let valid = self.valid_urls();
        let have = (if self.list_mode { &self.src_urls } else { &valid }).contains(&url) || valid.contains(&url);
        if !have {
            if self.running || self.preparing {
                // идёт загрузка — ссылка ждёт в списке и скачается следующим запуском
                let cur = self.text.trim_end().to_string();
                let new = if cur.is_empty() { format!("{url}\n") } else { format!("{cur}\n{url}\n") };
                self.set_text(new);
                self.flash_hint(self.tr("ext.queued"));
            } else {
                self.append_text(&url);
            }
        } else if !self.running {
            self.shown_url = None;
            self.show_preview_for(Some(url.clone()), false);
        }
        self.append_log(&self.trp("ext.got", &[("url", &url)]), 0);
    }

    // ------------------------------------------------------------ строки прогресса

    fn dl_steps(&self, r: &RowSt) -> Vec<(&'static str, String)> {
        if r.audio_only { return vec![("audio", self.tr("step.audio"))]; }
        let mut list = vec![];
        if r.has_subs { list.push(("subs", self.tr("step.subs"))); }
        list.push(("video", self.tr("step.video")));
        let an = r.audio_n.max(1);
        list.push(("audio", if an > 1 {
            if r.step.as_deref() == Some("audio") { self.trp("step.audioOf", &[("i", &r.audio_i.max(1).to_string()), ("n", &an.to_string())]) } else { self.trp("step.audioX", &[("n", &an.to_string())]) }
        } else { self.tr("step.audio") }));
        list.push(("merge", self.tr("step.merge")));
        list
    }

    fn row_view(&self, r: &RowSt) -> PRow {
        let pct = if r.state == Rs::Done { 100.0 } else { r.pct };
        let pct_text = if matches!(r.state, Rs::Pending | Rs::Error) { "—".to_string() } else { format!("{}{NB}%", pct.floor() as i64) };
        let mut steps = vec![];
        if r.step.is_some() && r.state != Rs::Pending {
            let list = self.dl_steps(r);
            let ci = if r.state == Rs::Done { list.len() } else { list.iter().position(|x| Some(x.0) == r.step.as_deref()).unwrap_or(usize::MAX) };
            steps = list.into_iter().enumerate().map(|(i, (_, t))| Step { text: ss(t), cls: if ci == usize::MAX || i > ci { 2 } else if i < ci { 0 } else { 1 } }).collect();
        }
        let num = if self.total > 1 && r.state != Rs::Pending { r.idx.map(|i| self.trp("row.item", &[("i", &(i + 1).to_string())])).unwrap_or_default() } else { String::new() };
        PRow {
            label: ss(&r.label),
            num: ss(num),
            check: r.state == Rs::Done,
            steps: model(steps),
            pct_text: ss(pct_text),
            pct: if r.state == Rs::Pending { 0.0 } else { pct as f32 },
            state: match r.state { Rs::Pending => 0, Rs::Active => 1, Rs::Done => 2, Rs::Error => 3 },
            l: ss(&r.l),
            r: ss(if self.paused && r.state == Rs::Active { self.tr("prog.paused") } else { r.r.clone() }),
            r_tip: SharedString::default(),
        }
    }

    fn render_rows(&self) {
        let mut rows = vec![self.row_view(&self.dl)];
        if self.settings_at_run.codec != "copy" || self.tc.state != Rs::Pending {
            rows.push(self.row_view(&self.tc));
        }
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_rows(model(rows));
        s.set_rows_show(self.rows_shown);
    }

    fn set_title(&self, t: &str) {
        self.ui().global::<S>().set_title(if t.is_empty() { "To Picture".into() } else { format!("{t} — To Picture").into() });
    }

    fn update_title(&self) {
        if !self.running { return self.set_title(""); }
        let act = if self.tc.state == Rs::Active { Some(&self.tc) } else if self.dl.state == Rs::Active { Some(&self.dl) } else { None };
        match act {
            None => self.set_title(""),
            Some(a) => self.set_title(&if self.paused { self.tr("prog.paused") } else { format!("{}{NB}% · {}", a.pct.floor() as i64, a.label) }),
        }
    }

    fn next_label(&self) -> String {
        let i = (self.cur + 1) as usize;
        if i >= self.total { return String::new(); }
        let u = &self.run_urls[i];
        let name = match self.resolved.get(u).and_then(|d| d.title()) {
            Some(t) => t.to_string(),
            None => self.run_labels.get(i).filter(|l| !l.is_empty()).cloned().unwrap_or_else(|| u.clone()),
        };
        self.trp("q.next", &[("name", &name)])
    }

    fn render_queue_label(&self) {
        if !self.running || self.cur < 0 { return; }
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_q_label(self.trp("q.videoOf", &[("i", &(self.cur + 1).to_string()), ("n", &self.total.to_string())]).into());
        s.set_q_next(self.next_label().into());
    }

    // ------------------------------------------------------------ запуск

    fn start(&mut self) {
        if self.running || self.preparing { return; }
        let urls = if self.list_mode { self.src_urls.clone() } else { self.valid_urls() };
        if urls.is_empty() || !self.missing.is_empty() { return; }
        self.ui().invoke_close_popups();
        self.preparing = true;
        self.prep_urls = urls;
        // списки видео плейлистов нужны до запуска (обычные ссылки не ждём)
        for u in self.prep_urls.clone() {
            if !self.done.contains_key(&u) && looks_like_playlist(&u) {
                self.get_preview(&u);
            }
        }
        self.update_buttons();
        self.try_start();
    }

    fn try_start(&mut self) {
        let urls = self.prep_urls.clone();
        if urls.iter().any(|u| !self.done.contains_key(u) && looks_like_playlist(u)) {
            return; // ждём превью плейлистов
        }
        self.preparing = false;
        let mut items: Vec<RunItem> = vec![];
        for u in &urls {
            match self.done.get(u) {
                Some(PvData::Playlist(d)) => {
                    let sel = self.pl_sel.get(u);
                    let pad = d.entries.len().to_string().len().max(2);
                    for (i, e) in d.entries.iter().enumerate() {
                        if sel.is_none_or(|s| s.contains(&i)) {
                            items.push(RunItem { url: e.url.clone(), folder: d.title.clone(), prefix: format!("{:0pad$} ", i + 1), title: e.title.clone() });
                        }
                    }
                }
                _ => items.push(RunItem { url: u.clone(), ..Default::default() }),
            }
        }
        if items.is_empty() {
            self.update_buttons();
            return;
        }
        self.running = true;
        self.cancelling = false;
        self.paused = false;
        self.src_urls = urls.clone();
        self.run_urls = items.iter().map(|x| x.url.clone()).collect();
        self.run_labels = items.iter().map(|x| if x.title.is_empty() { String::new() } else { format!("{}{}", x.prefix, x.title) }).collect();
        self.line_state = vec![Ls::Pending; items.len()];
        self.plans.clear();
        self.cur = -1;
        self.total = items.len();
        self.settings_at_run = self.s.clone();
        self.last_file = None;
        self.resolved.clear();
        self.run_items = items.clone();
        // для обычных ссылок превью уже есть или грузится; видео плейлиста получают его по ходу загрузки
        for it in &items {
            if it.folder.is_empty() {
                if let Some(d) = self.done.get(&it.url) { self.resolved.insert(it.url.clone(), d.clone()); } else { self.get_preview(&it.url); }
            }
        }
        self.dl = RowSt { label: self.tr("prog.download"), ..Default::default() };
        self.tc = RowSt { label: self.tr("prog.transcode"), ..Default::default() };
        self.set_list_mode(true);
        self.lock_settings(true);
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_prog_show(true);
        s.set_q_label("".into());
        s.set_q_next("".into());
        s.set_res_show(false);
        s.set_err_show(false);
        s.set_running(true);
        self.rows_shown = false;
        self.render_rows();
        self.update_buttons();
        let mut overrides = HashMap::new();
        for u in &urls {
            let mut ov = Override::default();
            if let Some(a) = self.audio_ov.get(u) {
                ov.audio = Some(match a { AudioOv::Rule(r) => r.clone(), AudioOv::Langs(l) => l.join(",") });
            }
            ov.trim = self.trim.get(u).copied();
            if ov.audio.is_some() || ov.trim.is_some() {
                overrides.insert(u.clone(), ov);
            }
        }
        let list = items.into_iter().map(|x| Item { url: x.url, folder: x.folder, prefix: x.prefix }).collect();
        if !self.be.start(list, self.s.clone(), overrides) {
            self.running = false;
            self.ui().global::<S>().set_running(false);
            self.lock_settings(false);
            self.update_buttons();
        }
    }

    fn toggle_pause(&mut self) {
        if !self.running || self.cancelling { return; }
        self.be.pause(!self.paused);
    }

    fn cancel(&mut self) {
        if !self.running || self.cancelling { return; }
        self.cancelling = true;
        self.update_buttons();
        self.be.cancel();
    }

    // ------------------------------------------------------------ события движка

    pub fn on_event(&mut self, ev: Event) {
        match ev {
            Event::Log { line, level } => {
                let lv = match level { LogLevel::Error => 2, LogLevel::Command => 1, _ => 0 };
                self.append_log(&line, lv);
            }
            Event::Paused { on } => {
                self.paused = on;
                self.render_rows();
                self.update_title();
                self.update_buttons();
            }
            Event::Item { index, total } => self.ev_item(index, total),
            Event::Meta { index, data } => {
                let Some(url) = self.run_urls.get(index).cloned() else { return };
                if let Some(px) = data.thumb.as_ref().and_then(|b| decode_thumb(b)) {
                    self.thumbs.insert(url.clone(), Image::from_rgba8(px));
                }
                let mut data = data;
                data.url = url.clone();
                let d = PvData::Video(Arc::new(data));
                self.done.insert(url.clone(), d.clone());
                self.resolved.insert(url.clone(), d.clone());
                if self.dl.idx == Some(index) {
                    self.shown_url = Some(url.clone());
                    self.shown = Some((url, d.clone()));
                    self.render_preview(&d);
                }
            }
            Event::Plan { index, subs, audio_n, copy, audio_only } => {
                self.plans.insert(index, (copy, audio_only));
                if self.dl.idx != Some(index) { return; }
                self.dl.has_subs = subs;
                self.dl.audio_n = audio_n;
                self.dl.audio_only = audio_only;
                self.render_rows();
            }
            Event::TcStart { index } => {
                let (pc, _) = self.plans.get(&index).copied().unwrap_or_default();
                let copy = pc || self.settings_at_run.codec == "copy";
                self.rows_shown = true;
                self.tc = RowSt { label: self.tr(if copy { "prog.remux" } else { "prog.transcode" }), idx: Some(index), copy, state: Rs::Active, l: self.tr("prog.ffmpegStart"), ..Default::default() };
                self.render_rows();
                self.update_title();
            }
            Event::Progress(p) => self.ev_progress(p),
            Event::StageDone { index, stage, size, elapsed, merged } => {
                self.rows_shown = true;
                if stage == Stage::Download {
                    let d = &self.dl;
                    self.dl = RowSt {
                        label: self.tr("prog.download"), idx: Some(index), state: Rs::Done, step: Some("merge".into()),
                        audio_n: d.audio_n, has_subs: d.has_subs, audio_only: d.audio_only,
                        l: fmt::size(&self.lang, size) + &if merged { self.tr("prog.merged") } else { String::new() },
                        r: fmt::time(elapsed), ..Default::default()
                    };
                } else {
                    let copy = self.tc.copy;
                    self.tc = RowSt {
                        label: self.tr(if copy { "prog.remux" } else { "prog.transcode" }), idx: Some(index), state: Rs::Done, copy,
                        l: format!("{} · {}", fmt::size(&self.lang, size), if copy { self.tr("prog.remuxDone") } else { self.codec_desc(&self.settings_at_run.clone()) }),
                        r: fmt::time(elapsed), ..Default::default()
                    };
                }
                self.render_rows();
                self.update_title();
            }
            Event::TcMode { index, copy } => {
                if self.tc.idx != Some(index) { return; }
                if copy {
                    let ao = self.plans.get(&index).map(|p| p.1).unwrap_or(false);
                    self.tc.label = self.tr("prog.remux");
                    self.tc.copy = true;
                    self.tc.l = self.tr(if ao { "prog.remuxAudio" } else { "prog.remuxNote" });
                    self.render_rows();
                }
            }
            Event::ItemDone { index, path, size, srt } => {
                if let Some(x) = self.line_state.get_mut(index) { *x = Ls::Done; }
                self.last_file = Some(path);
                self.last_size = size;
                self.last_srt = srt;
                self.render_gutter();
                self.render_url_list();
                self.update_buttons();
            }
            Event::ItemError { index, total, message, stage } => {
                if let Some(x) = self.line_state.get_mut(index) { *x = Ls::Error; }
                self.render_gutter();
                self.render_url_list();
                let failing = if stage == Stage::Transcode { &mut self.tc } else { &mut self.dl };
                failing.state = Rs::Error;
                failing.l = message.clone();
                failing.r.clear();
                let label = failing.label.clone();
                self.render_rows();
                let ui = self.ui();
        let s = ui.global::<S>();
                s.set_err_t(self.trp("err.interrupted", &[("stage", &label), ("i", &(index + 1).to_string()), ("n", &total.to_string())]).into());
                s.set_err_m(message.into());
                s.set_err_show(true);
                self.set_log_open(true);
            }
            Event::ItemCancelled { index, stage } => {
                if let Some(x) = self.line_state.get_mut(index) { *x = Ls::Pending; }
                self.render_gutter();
                self.render_url_list();
                let label = self.tr("prog.cancelled");
                let r = if stage == Stage::Transcode { &mut self.tc } else { &mut self.dl };
                if r.idx != Some(index) || r.state == Rs::Done { return; }
                r.state = Rs::Error;
                r.l = label;
                r.r.clear();
                self.render_rows();
            }
            Event::Finished { saved, failed, total, bytes, elapsed, cancelled } => self.ev_finished(saved, failed, total, bytes, elapsed, cancelled),
        }
    }

    fn ev_item(&mut self, index: usize, total: usize) {
        self.cur = index as isize;
        if let Some(x) = self.line_state.get_mut(index) { *x = Ls::Active; }
        self.render_gutter();
        self.render_url_list();
        self.dl = RowSt { label: self.tr("prog.download"), idx: Some(index), state: Rs::Active, step: Some("video".into()), l: self.tr("prog.connecting"), ..Default::default() };
        // строка перекодирования показывает следующее видео, только если сейчас она свободна
        if self.tc.state != Rs::Active {
            let desc = self.codec_desc(&self.settings_at_run.clone());
            self.tc = RowSt { label: self.tr("prog.transcode"), idx: Some(index), l: self.trp("prog.waiting", &[("desc", &desc)]), ..Default::default() };
            // если по превью уже понятно, что сайт отдаёт H.264, — честно пишем «перепаковка»
            if let Some(PvData::Video(v)) = self.run_urls.get(index).and_then(|u| self.resolved.get(u)).cloned() {
                let mut v = (*v).clone();
                v.url = self.run_urls[index].clone();
                if self.select_video(&v).1 {
                    self.tc = RowSt { label: self.tr("prog.remux"), idx: Some(index), copy: true, l: self.trp("prog.waiting", &[("desc", &self.tr("desc.noReencode"))]), ..Default::default() };
                }
            }
            self.rows_shown = false;
        }
        self.render_rows();
        self.total = total;
        self.render_queue_label();
        let it = self.run_items.get(index).cloned().unwrap_or_default();
        if !it.folder.is_empty() && !self.done.contains_key(&it.url) {
            // видео из плейлиста: превью придёт событием meta
            self.shown_url = Some(it.url.clone());
            self.shown = None;
            self.pv_gen += 1;
            self.render_preview_state_loading(&it.url);
        } else {
            let u = self.run_urls.get(index).cloned();
            self.show_preview_for(u, false);
        }
        self.update_title();
    }

    fn ev_progress(&mut self, p: Progress) {
        self.rows_shown = true;
        let lang = self.lang.clone();
        if p.stage == Some(Stage::Download) {
            if self.dl.idx != Some(p.index) { return; }
            let merging = self.tr("prog.merging");
            let left = p.eta.map(|e| self.trp("prog.left", &[("t", &fmt::time(e))]));
            let r = &mut self.dl;
            r.state = Rs::Active;
            r.step = p.step.clone();
            if let Some(n) = p.audio_n { r.audio_n = n; r.audio_i = p.audio_i.unwrap_or(1); }
            if let Some(note) = p.note {
                r.l = note;
                r.r.clear();
                if let Some(x) = p.pct { r.pct = x; }
            } else if p.step.as_deref() == Some("merge") {
                r.pct = 100.0;
                r.l = merging;
                r.r.clear();
            } else {
                if let Some(x) = p.pct { r.pct = x; }
                let mut parts = vec![];
                if let Some(t) = p.total.filter(|t| *t > 0) { parts.push(fmt::pair(&lang, p.done.unwrap_or(0), t)); }
                if let Some(sp) = p.speed.filter(|s| *s > 0.0) { parts.push(fmt::speed(&lang, sp)); }
                if !parts.is_empty() { r.l = parts.join(" · "); }
                r.r = left.unwrap_or_default();
            }
        } else {
            if self.tc.idx != Some(p.index) { return; }
            let mut parts = vec![];
            if let Some(fs) = p.frames.filter(|f| *f > 0) {
                parts.push(self.trp("prog.frameOf", &[("a", &fmt::group_num(&lang, p.frame.unwrap_or(0))), ("b", &fmt::group_num(&lang, fs))]));
            } else if let Some(f) = p.frame.filter(|f| *f > 0) {
                parts.push(self.trp("prog.frame", &[("a", &fmt::group_num(&lang, f))]));
            }
            if let Some(sp) = p.speed.filter(|s| *s > 0.0) { parts.push(fmt::nf(&lang, 1, sp) + "×"); }
            let left = p.eta.map(|e| self.trp("prog.left", &[("t", &fmt::time(e))])).unwrap_or_default();
            let r = &mut self.tc;
            r.state = Rs::Active;
            r.pct = p.pct.unwrap_or(r.pct);
            r.l = parts.join(" · ");
            r.r = left;
        }
        self.render_rows();
        self.update_title();
    }

    fn ev_finished(&mut self, saved: usize, failed: usize, total: usize, bytes: u64, elapsed: f64, cancelled: bool) {
        self.running = false;
        self.cancelling = false;
        self.paused = false;
        self.ui().global::<S>().set_running(false);
        self.lock_settings(false);
        let ui = self.ui();
        let s = ui.global::<S>();
        if cancelled {
            s.set_q_label(self.tr("q.cancelled").into());
            s.set_q_next(if saved > 0 { self.trp("q.saved", &[("n", &saved.to_string())]) } else { String::new() }.into());
        } else {
            let n = total.to_string();
            s.set_q_label(self.trp(if failed > 0 { "q.withErrors" } else { "q.done" }, &[("n", &n)]).into());
            let mut parts = vec![self.trp("q.saved", &[("n", &saved.to_string())])];
            if bytes > 0 { parts.push(fmt::size(&self.lang, bytes)); }
            parts.push(fmt::time(elapsed));
            if failed > 0 { parts.push(self.trp("q.errors", &[("n", &failed.to_string())])); }
            s.set_q_next(parts.join(" · ").into());
        }
        self.render_result();
        let line = if cancelled {
            self.tr("log.cancelled")
        } else {
            self.trp("log.done", &[("a", &saved.to_string()), ("b", &total.to_string())])
                + &if failed > 0 { self.trp("log.errors", &[("n", &failed.to_string())]) } else { self.tr("log.noErrors") }
        };
        self.append_log(&line, if failed > 0 { 2 } else { 0 });
        self.set_title("");
        self.render_rows();
        self.update_buttons();
        self.render_app_menu();
        self.shown_url = None;
        let last = self.src_urls.last().cloned();
        self.show_preview_for(last, false);
    }

    fn render_result(&self) {
        let ui = self.ui();
        let s = ui.global::<S>();
        let Some(f) = &self.last_file else { s.set_res_show(false); return };
        s.set_res_show(true);
        s.set_res_path(self.fit("resPath", f, 12.0, false).into());
        s.set_res_tip(f.clone().into());
        s.set_res_size(fmt::size(&self.lang, self.last_size).into());
        s.set_srt_show(!self.last_srt.is_empty());
        if let Some(first) = self.last_srt.first() {
            let name = first.rsplit(['\\', '/']).next().unwrap_or(first).to_string();
            let extra = if self.last_srt.len() > 1 { format!("  +{}", self.last_srt.len() - 1) } else { String::new() };
            s.set_srt_name(format!("{name}{extra}").into());
            s.set_srt_tip(self.last_srt.join("\n").into());
        }
    }

    // ------------------------------------------------------------ журнал

    fn append_log(&mut self, line: &str, level: i32) {
        self.log.push(LogLine { text: ss(line), level });
        if self.log.row_count() > 2000 { self.log.remove(0); }
        self.log_last = line.to_string();
        if !line.starts_with("$ ") {
            self.ui().global::<S>().set_log_sum(line.into());
        }
    }

    fn set_log_open(&mut self, open: bool) {
        self.log_open = open;
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_log_open(open);
        s.set_copy_label(self.tr("log.copy").into());
    }

    fn copy_log(&mut self) {
        let text = self.log.iter().map(|l| l.text.to_string()).collect::<Vec<_>>().join("\r\n");
        set_clipboard(&text);
        self.ui().global::<S>().set_copy_label(self.tr("log.copied").into());
        self.copy_timer.start(slint::TimerMode::SingleShot, Duration::from_millis(1200), || {
            with_app(|a| { let l = a.tr("log.copy"); a.ui().global::<S>().set_copy_label(l.into()); })
        });
    }

    fn flash_hint(&mut self, text: String) {
        self.hint_flash = text;
        self.update_buttons();
        self.hint_timer.start(slint::TimerMode::SingleShot, Duration::from_secs(4), || {
            with_app(|a| { a.hint_flash.clear(); a.update_buttons(); })
        });
    }

    // ------------------------------------------------------------ кнопки и подсказка

    fn update_buttons(&self) {
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_pause_show(self.running);
        s.set_pause_text(self.tr(if self.paused { "btn.resume" } else { "btn.pause" }).into());
        s.set_pause_dis(self.cancelling);
        if self.preparing {
            s.set_main_text(self.tr("btn.preparing").into());
            s.set_main_dis(true);
            s.set_main_cancel(false);
        } else if self.running {
            s.set_main_text(self.tr("btn.cancel").into());
            s.set_main_cancel(true);
            s.set_main_dis(self.cancelling);
        } else {
            s.set_main_text(self.tr("btn.download").into());
            s.set_main_cancel(false);
            let n = if self.list_mode { self.src_urls.len() } else { self.valid_urls().len() };
            s.set_main_dis(n == 0 || !self.missing.is_empty());
        }
        s.set_folder_dis(self.last_file.is_none());
        s.set_clear_dis(self.running || (!self.list_mode && self.text.trim().is_empty()));
        s.set_paste_dis(self.running);
        s.set_hint_err(!self.missing.is_empty());
        let hint = if !self.missing.is_empty() {
            self.trp("hint.missing", &[("list", &self.missing.join(", "))])
        } else if !self.hint_flash.is_empty() {
            self.hint_flash.clone()
        } else if self.running {
            String::new()
        } else if self.text.trim().is_empty() && !self.list_mode {
            self.tr("hint.keysEmpty")
        } else {
            self.tr("hint.keys")
        };
        s.set_hint(hint.into());
    }

    fn render_all(&mut self) {
        self.render_settings();
        self.render_gutter();
        if self.list_mode { self.render_url_list(); }
        self.update_buttons();
        self.render_app_menu();
        let ui = self.ui();
        let s = ui.global::<S>();
        s.set_log_open(self.log_open);
        s.set_copy_label(self.tr("log.copy").into());
        if self.running || self.total > 0 {
            // подписи строк прогресса — на текущем языке
            self.dl.label = self.tr("prog.download");
            self.tc.label = self.tr(if self.tc.copy { "prog.remux" } else { "prog.transcode" });
            self.render_rows();
            self.render_queue_label();
            self.render_result();
        }
        if s.get_ext_show() { self.render_ext(); }
        self.update_title();
    }

    fn fit_changed(&mut self, id: &str, w: f32) {
        let old = self.widths.insert(id.to_string(), w);
        if old.is_some_and(|o| (o - w).abs() < 0.5) { return; }
        match id {
            "win" => {
                let was = old.map(|o| o < 780.0);
                if was != Some(w < 780.0) { self.render_grid(); }
            }
            "outDir" | "cookiesFile" => {
                let ui = self.ui();
        let s = ui.global::<S>();
                s.set_out_dir(self.fit("outDir", &self.s.out_dir, 13.0, false).into());
                if !self.s.cookies_file.is_empty() {
                    s.set_cookies_file(self.fit("cookiesFile", &self.s.cookies_file, 13.0, false).into());
                }
            }
            "resPath" => self.render_result(),
            "extPath" => {
                let f = self.ext_folder.clone();
                self.ui().global::<S>().set_ext_path(self.fit("extPath", &f, 12.0, true).into());
            }
            _ => {}
        }
    }

    /// Окно показано: ссылки, пришедшие до этого, версии, видеокарта, связь с расширением.
    pub fn after_show(&mut self) {
        let ui = self.ui();
        self.fit_changed("win", ui.window().size().to_logical(ui.window().scale_factor()).width);
        self.apply_theme();
        if let Some(d) = self.dark { win::set_dark_title(ui.window(), d); }
        let be = self.be.clone();
        std::thread::spawn(move || {
            let v = be.versions();
            post(move |a| {
                a.versions = v.clone();
                a.log.insert(0, LogLine { text: ss(&v), level: 0 });
                if a.log_last.is_empty() { a.ui().global::<S>().set_log_sum(v.into()); }
            });
            // поиск видеокарты стартует вместе с проверкой версий — спрашиваем после
            let g = be.gpu_info();
            post(move |a| {
                a.gpu = g;
                a.render_settings();
            });
        });
        self.be.start_ipc(Box::new(|| post(|a| a.take_inbox(true))));
        self.take_inbox(false);
        if self.be.should_show_help_once() {
            self.open_ext();
        }
    }

    pub fn closing(&mut self) {
        self.be.stop_ipc();
        self.be.shutdown();
    }
}

// ---------------------------------------------------------------- вспомогательное

fn decode_thumb(bytes: &[u8]) -> Option<SharedPixelBuffer<Rgba8Pixel>> {
    let img = image::load_from_memory(bytes).ok()?;
    // карточка 240×135 — хватит 480 px по ширине (чёткость на 200 %)
    let img = if img.width() > 480 { img.resize(480, 480 * img.height() / img.width().max(1), image::imageops::FilterType::Triangle) } else { img };
    let rgba = img.to_rgba8();
    Some(SharedPixelBuffer::clone_from_slice(rgba.as_raw(), rgba.width(), rgba.height()))
}

pub fn get_clipboard() -> String {
    arboard::Clipboard::new().and_then(|mut c| c.get_text()).unwrap_or_default()
}

pub fn set_clipboard(text: &str) {
    if let Ok(mut c) = arboard::Clipboard::new() {
        let _ = c.set_text(text.to_string());
    }
}

/// Подключить обработчики интерфейса к состоянию.
fn wire(ui: &AppWindow) {
    let s = ui.global::<S>();
    s.on_tr(|k, lang| t(&lang, &k, &[]).into());
    ui.global::<Measure>().on_text_w(|text, px, weight| fmt::text_width_w(&text, px, weight >= 600).ceil() + 2.0);
    s.on_urls_edited(|text| with_app(move |a| { a.text = text.to_string(); a.on_list_changed(); }));
    s.on_edit_list(|| with_app(|a| a.edit_list()));
    s.on_paste(|| with_app(|a| { if a.running { return; } let t = get_clipboard(); a.append_text(&t); }));
    s.on_key_paste(|| with_app(|a| { if a.running { return; } let t = get_clipboard(); a.append_text(&t); }));
    s.on_clear(|| with_app(|a| {
        if a.list_mode { a.set_list_mode(false); }
        a.set_text(String::new());
        a.on_list_changed();
        a.ui().invoke_focus_urls();
    }));
    s.on_app_act(|id| {
        let mut r = "close";
        let rc = APP.with(|a| a.borrow().clone());
        if let Some(rc) = rc
            && let Ok(mut a) = rc.try_borrow_mut()
        {
            r = a.app_act(&id);
        }
        r.into()
    });
    s.on_lang_act(|code| with_app(move |a| a.lang_act(&code)));
    s.on_select(|k, v| with_app(move |a| {
        if a.locked { return; }
        a.set(&k, v.to_string());
        a.on_settings_changed();
        if k == "cookies" { a.on_net_changed(); }
    }));
    s.on_crf_set(|v| with_app(move |a| { if a.locked { return; } a.s.crf = (v as i64).clamp(10, 30); a.on_settings_changed(); }));
    s.on_browse(|| with_app(|a| {
        let ui = a.ui();
        let mut d = rfd::FileDialog::new().set_parent(&ui.window().window_handle());
        if std::path::Path::new(&a.s.out_dir).is_dir() { d = d.set_directory(&a.s.out_dir); }
        if let Some(p) = d.pick_folder() {
            a.s.out_dir = p.to_string_lossy().into_owned();
            a.on_settings_changed();
        }
    }));
    s.on_browse_cookies(|| with_app(|a| {
        let ui = a.ui();
        let mut d = rfd::FileDialog::new().set_parent(&ui.window().window_handle()).add_filter("cookies", &["txt"]).add_filter("All files", &["*"]);
        if let Some(dir) = std::path::Path::new(&a.s.cookies_file).parent().filter(|p| p.is_dir()) { d = d.set_directory(dir); }
        if let Some(p) = d.pick_file() {
            a.s.cookies_file = p.to_string_lossy().into_owned();
            a.on_settings_changed();
            a.on_net_changed();
        }
    }));
    s.on_proxy_edited(|t| with_app(move |a| {
        a.widths.insert("proxyFocus".into(), 1.0);
        a.s.proxy = t.trim().to_string();
        a.on_settings_changed();
    }));
    s.on_proxy_done(|| with_app(|a| { a.widths.remove("proxyFocus"); a.on_net_changed(); }));
    s.on_toggle_settings(|v| with_app(move |a| { a.s.settings_open = v; a.on_settings_changed(); }));
    s.on_fit(|id, w| with_app(move |a| a.fit_changed(&id, w)));
    s.on_track_open(|| with_app(|a| a.render_track_menu()));
    s.on_track_click(|id| {
        let rc = APP.with(|a| a.borrow().clone());
        rc.and_then(|rc| rc.try_borrow_mut().ok().map(|mut a| a.track_click(&id))).unwrap_or(true)
    });
    s.on_audio_reset(|| with_app(|a| {
        if let Some(d) = a.shown_video() { a.audio_ov.remove(&d.url); a.render_video(&d); }
    }));
    s.on_pl_open(|| with_app(|a| {
        if let Some((u, PvData::Playlist(d))) = a.shown.clone() { a.render_pl_items(&d, &u); }
    }));
    s.on_pl_click(|id| with_app(move |a| a.pl_click(&id)));
    s.on_subs_open(|| with_app(|a| a.render_subs_pop()));
    s.on_trim_open(|| with_app(|a| a.trim_open()));
    s.on_trim_apply(|x, y| {
        let rc = APP.with(|a| a.borrow().clone());
        rc.and_then(|rc| rc.try_borrow_mut().ok().map(|mut a| a.trim_apply(&x, &y))).unwrap_or(false)
    });
    s.on_trim_all(|| with_app(|a| {
        if let Some(d) = a.shown_video() { a.trim.remove(&d.url); a.render_video(&d); }
    }));
    s.on_toggle_log(|v| with_app(move |a| a.set_log_open(v)));
    s.on_copy_log(|| with_app(|a| a.copy_log()));
    s.on_main(|| with_app(|a| if a.running { a.cancel() } else { a.start() }));
    s.on_key_start(|| with_app(|a| a.start()));
    s.on_pause(|| with_app(|a| a.toggle_pause()));
    s.on_key_pause(|| with_app(|a| a.toggle_pause()));
    s.on_folder(|| with_app(|a| { if let Some(f) = &a.last_file { win::show_in_folder(f); } }));
    s.on_copy_result(|| with_app(|a| { if let Some(f) = &a.last_file { set_clipboard(f); } }));
    s.on_ext_close(|| with_app(|a| { a.ui().global::<S>().set_ext_show(false); a.ui().invoke_focus_urls(); }));
    s.on_ext_connect(|| with_app(|a| { a.be.ext_register(); a.render_ext(); }));
    s.on_ext_copy(|id| with_app(move |a| a.ext_copy(&id)));
    s.on_ext_open(|| with_app(|a| { if std::path::Path::new(&a.ext_folder).is_dir() { win::open(&a.ext_folder.clone()); } }));
    s.on_theme_changed(|_| with_app(|a| a.apply_theme()));
}

impl App {
    /// Демо-сценарий для снимков окна (TOPICTURE_DEMO_AUTO).
    pub fn demo_action(&mut self, act: &str) {
        let (k, v) = act.split_once('=').unwrap_or((act, ""));
        match k {
            "paste" => { let t = v.replace('|', "\n"); self.append_text(&t); }
            "settings" => { self.s.settings_open = v != "0"; self.on_settings_changed(); }
            "set" => {
                if let Some((key, val)) = v.split_once(',') {
                    if key == "cookies_file" { self.s.cookies_file = val.into(); } else { self.set(key, val.into()); }
                    self.on_settings_changed();
                }
            }
            "start" => self.start(),
            "pause" => self.toggle_pause(),
            "cancel" => self.cancel(),
            "log" => { self.set_log_open(v != "0"); }
            "ext" => self.open_ext(),
            "theme" => { self.s.theme = v.into(); self.on_settings_changed(); }
            "lang" => self.lang_act(v),
            "trim" => { if let Some(d) = self.shown_video() { self.trim.insert(d.url.clone(), (75.0, Some(330.0))); self.render_video(&d); } }
            "audio" => { if let Some(d) = self.shown_video() { self.audio_ov.insert(d.url.clone(), AudioOv::Langs(vec!["en".into(), "de".into()])); self.render_video(&d); } }
            "popup" => {
                match v {
                    "track" => self.render_track_menu(),
                    "subs" => self.render_subs_pop(),
                    "trim" => self.trim_open(),
                    "pl" => { if let Some((u, PvData::Playlist(d))) = self.shown.clone() { self.render_pl_items(&d, &u); } }
                    _ => {}
                }
                self.ui().invoke_demo_popup(v.into());
            }
            "inbox" => self.take_inbox(true),
            _ => {}
        }
    }
}
