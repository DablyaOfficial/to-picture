//! Демо-режим (TOPICTURE_DEMO=1): ненастоящий движок для разработки и снимков окна.
//! Превью видео и плейлиста, сценарий загрузки с паузой, ошибкой и завершением — без сети и yt-dlp.
use crate::backend::Backend;
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::sleep;
use std::time::Duration;
use topicture_bridge as bridge;
use topicture_core::*;

pub struct Demo {
    sink: EventSink,
    running: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    cancel: Arc<AtomicBool>,
    registered: AtomicBool,
    inbox_sent: AtomicBool,
    settings: Mutex<Settings>,
}

impl Demo {
    pub fn new(sink: EventSink) -> Demo {
        Demo {
            sink,
            running: Default::default(),
            paused: Default::default(),
            cancel: Default::default(),
            registered: AtomicBool::new(std::env::var("TOPICTURE_DEMO_EXT").is_ok_and(|v| v == "1")),
            inbox_sent: Default::default(),
            settings: Mutex::new(Settings::default()),
        }
    }
}

/// Скорость сценария: TOPICTURE_DEMO_SPEED=2 — вдвое быстрее.
fn tick() -> Duration {
    let k: f64 = std::env::var("TOPICTURE_DEMO_SPEED").ok().and_then(|v| v.parse().ok()).unwrap_or(1.0);
    Duration::from_millis((120.0 / k.max(0.05)) as u64)
}

fn thumb(seed: u32) -> Option<Arc<Vec<u8>>> {
    let (w, h) = (480u32, 270u32);
    let mut img = image::RgbImage::new(w, h);
    let (a, b) = match seed % 3 {
        0 => ([38u8, 70, 110], [214u8, 120, 60]),
        1 => ([30, 90, 70], [200, 190, 90]),
        _ => ([80, 40, 100], [230, 140, 160]),
    };
    for (x, y, p) in img.enumerate_pixels_mut() {
        let t = (x as f32 / w as f32) * 0.7 + (y as f32 / h as f32) * 0.3;
        let mut c = [0u8; 3];
        for i in 0..3 {
            c[i] = (a[i] as f32 * (1.0 - t) + b[i] as f32 * t) as u8;
        }
        // «солнце» и «горы»
        let (dx, dy) = (x as f32 - 330.0, y as f32 - 95.0);
        if dx * dx + dy * dy < 40.0 * 40.0 {
            c = [250, 225, 170];
        }
        let ridge = 190.0 - 50.0 * ((x as f32) / 70.0).sin().abs() - (x as f32 / 9.0).sin() * 8.0;
        if (y as f32) > ridge {
            c = [c[0] / 3, c[1] / 3, c[2] / 3];
        }
        *p = image::Rgb(c);
    }
    let mut out = std::io::Cursor::new(Vec::new());
    img.write_to(&mut out, image::ImageFormat::Png).ok()?;
    Some(Arc::new(out.into_inner()))
}

fn video(url: &str) -> VideoPreview {
    let other = url.contains("vimeo");
    let tr = |l: &str, o: bool, a: bool| AudioTrack { lang: l.into(), original: o, auto: a };
    let f = |h: u32, fps: f64, vc: &str, hdr: bool| VideoFormat { h, fps: Some(fps), vc: vc.into(), hdr, https: true, tbr: Some(h as f64 * 4.0) };
    if other {
        return VideoPreview {
            url: url.into(),
            title: "Short film — a walk along the northern coast at dawn".into(),
            channel: "Vimeo".into(),
            duration: Some(412.0),
            views: Some(18_400),
            date: Some("20240512".into()),
            thumb: thumb(1),
            height: Some(1080),
            fps: Some(25.0),
            vcodec: Some("avc1.640028".into()),
            heights: vec![1080, 720, 540],
            vfmts: vec![f(1080, 25.0, "avc1.640028", false), f(720, 25.0, "avc1.4d401f", false)],
            tracks: vec![tr("und", true, false)],
            site: "vimeo.com".into(),
            youtube: false,
            ..Default::default()
        };
    }
    VideoPreview {
        url: url.into(),
        title: "How the James Webb Space Telescope sees the universe — a 4K documentary about light, dust and time".into(),
        channel: "Deep Sky Studio".into(),
        duration: Some(1874.0),
        views: Some(2_437_118),
        date: Some("20250314".into()),
        thumb: thumb(0),
        height: Some(2160),
        fps: Some(60.0),
        vcodec: Some("vp09.00.51.08".into()),
        heights: vec![2160, 1440, 1080, 720, 480],
        vfmts: vec![
            f(2160, 60.0, "vp09.00.51.08", false),
            f(2160, 60.0, "vp09.02.51.10", true),
            f(1440, 60.0, "vp09.00.50.08", false),
            f(1080, 60.0, "avc1.64002a", false),
            f(1080, 60.0, "vp09.00.41.08", false),
            f(720, 60.0, "avc1.4d4020", false),
        ],
        tracks: vec![tr("en", true, false), tr("ru", false, false), tr("de", false, true), tr("fr", false, true), tr("es", false, true), tr("ja", false, true)],
        subs: ["en", "ru", "de", "fr", "es", "ja", "pt-BR", "zh-Hans", "ko", "tr", "it", "pl"].iter().map(|s| s.to_string()).collect(),
        auto_subs: true,
        language: Some("en".into()),
        site: "youtube.com".into(),
        youtube: true,
        ..Default::default()
    }
}

fn playlist(url: &str) -> PlaylistPreview {
    let titles = [
        "Episode 1 — The first light", "Episode 2 — Mirrors in the cold", "Episode 3 — Dust and nurseries of stars",
        "Episode 4 — Exoplanet atmospheres", "Episode 5 — The early galaxies", "Episode 6 — Black holes at cosmic dawn",
        "Episode 7 — Comets, rings and moons", "Episode 8 — Deep fields", "Episode 9 — How the images are colored",
        "Episode 10 — The engineering of the sunshield", "Episode 11 — Questions from viewers", "Episode 12 — What comes next",
    ];
    let entries: Vec<PlaylistEntry> = titles.iter().enumerate()
        .map(|(i, t)| PlaylistEntry { url: format!("https://www.youtube.com/watch?v=demo{i:02}"), title: t.to_string(), duration: Some(600.0 + i as f64 * 37.0) })
        .collect();
    let duration = entries.iter().filter_map(|e| e.duration).sum();
    PlaylistPreview {
        url: url.into(),
        title: "Webb Telescope — full series".into(),
        channel: "Deep Sky Studio".into(),
        site: "youtube.com".into(),
        youtube: true,
        thumb: thumb(2),
        entries,
        duration: Some(duration),
        ..Default::default()
    }
}

impl Backend for Demo {
    fn init(&self) -> InitInfo {
        let s = Settings::load();
        *self.settings.lock().unwrap() = s.clone();
        InitInfo {
            lang: i18n::resolve_lang(&s.lang),
            system_lang: i18n::system_lang(),
            settings: s,
            missing: std::env::var("TOPICTURE_DEMO_MISSING").ok().map(|v| v.split(',').map(String::from).collect()).unwrap_or_default(),
            portable: false,
        }
    }
    fn save_settings(&self, s: &Settings) {
        s.save();
        *self.settings.lock().unwrap() = s.clone();
    }
    fn preview(&self, url: &str) -> PreviewResult {
        sleep(Duration::from_millis(if std::env::var("TOPICTURE_DEMO_SLOWPV").is_ok() { 60_000 } else { 700 }));
        if url.contains("private") || url.contains("fail-preview") {
            let lang = i18n::resolve_lang(&self.settings.lock().unwrap().lang);
            return PreviewResult::Error { error: i18n::tr(&lang, "py.err.private", &[]), raw: "ERROR: Private video".into() };
        }
        if url.contains("list=") || url.contains("/playlist") {
            return PreviewResult::Playlist(playlist(url));
        }
        PreviewResult::Video(video(url))
    }
    fn start(&self, items: Vec<Item>, settings: Settings, _overrides: HashMap<String, Override>) -> bool {
        if self.running.swap(true, Ordering::SeqCst) {
            return false;
        }
        self.save_settings(&settings);
        self.paused.store(false, Ordering::SeqCst);
        self.cancel.store(false, Ordering::SeqCst);
        let (sink, running, paused, cancel) = (self.sink.clone(), self.running.clone(), self.paused.clone(), self.cancel.clone());
        std::thread::spawn(move || {
            script(&sink, &items, &settings, &paused, &cancel);
            running.store(false, Ordering::SeqCst);
        });
        true
    }
    fn pause(&self, on: bool) {
        if self.running.load(Ordering::SeqCst) {
            self.paused.store(on, Ordering::SeqCst);
            (self.sink)(Event::Paused { on });
        }
    }
    fn cancel(&self) { self.cancel.store(true, Ordering::SeqCst); }
    fn gpu_info(&self) -> Option<GpuInfo> {
        sleep(Duration::from_millis(600));
        if std::env::var("TOPICTURE_DEMO_NOGPU").is_ok() { None } else { Some(GpuInfo { vendor: "NVIDIA".into(), short: "NVENC".into() }) }
    }
    fn versions(&self) -> String {
        sleep(Duration::from_millis(200));
        format!("To Picture {APP_VERSION} · yt-dlp 2025.10.22 · ffmpeg 8.0 (demo)")
    }
    fn shutdown(&self) { self.cancel.store(true, Ordering::SeqCst); }
    fn put_cookies(&self, _site: String, _netscape: String) {}
    fn ext_status(&self) -> bridge::ExtStatus {
        bridge::ExtStatus {
            registered: self.registered.load(Ordering::SeqCst),
            folder: paths::extension_dir().to_string_lossy().into_owned(),
            has_folder: true,
            seen: false,
            portable: false,
            id: bridge::EXT_ID.into(),
        }
    }
    fn ext_register(&self) -> bool {
        self.registered.store(true, Ordering::SeqCst);
        true
    }
    fn take_inbox(&self) -> Vec<bridge::InboxLink> {
        match std::env::var("TOPICTURE_DEMO_INBOX") {
            Ok(url) if !self.inbox_sent.swap(true, Ordering::SeqCst) => {
                vec![bridge::InboxLink { url, title: String::new(), cookies: Some("# Netscape HTTP Cookie File\n".into()) }]
            }
            _ => vec![],
        }
    }
    fn should_show_help_once(&self) -> bool { std::env::var("TOPICTURE_DEMO_HELP").is_ok_and(|v| v == "1") }
    fn start_ipc(&self, _on_wake: Box<dyn Fn() + Send + Sync>) {}
    fn stop_ipc(&self) {}
}

/// Ждать тик; во время паузы — стоять. true — отменено.
fn wait(paused: &AtomicBool, cancel: &AtomicBool) -> bool {
    loop {
        sleep(tick());
        if cancel.load(Ordering::SeqCst) {
            return true;
        }
        if !paused.load(Ordering::SeqCst) {
            return false;
        }
    }
}

fn script(sink: &EventSink, items: &[Item], s: &Settings, paused: &AtomicBool, cancel: &AtomicBool) {
    let emit = |e: Event| sink(e);
    let log = |l: &str, level: LogLevel| emit(Event::Log { line: l.into(), level });
    let total = items.len();
    let (mut saved, mut failed, mut bytes) = (0usize, 0usize, 0u64);
    let started = std::time::Instant::now();
    let audio_only = s.is_audio_out();
    for (index, it) in items.iter().enumerate() {
        emit(Event::Item { index, total });
        log(&format!("$ yt-dlp -f bv*+ba/b --no-playlist -- {}", it.url), LogLevel::Command);
        if !it.folder.is_empty() {
            let mut v = video(&it.url);
            v.title = format!("{}{}", it.prefix, "Webb Telescope — episode");
            v.thumb = thumb(index as u32);
            emit(Event::Meta { index, data: v });
        }
        let audio_n = if audio_only { 1 } else { 2 };
        emit(Event::Plan { index, subs: !audio_only, audio_n, copy: false, audio_only });
        let fail = it.url.contains("fail");
        let steps: Vec<(&str, u64)> = if audio_only { vec![("audio", 9_800_000)] } else { vec![("subs", 0), ("video", 412_000_000), ("audio", 9_800_000), ("audio", 9_100_000), ("merge", 0)] };
        let mut ai = 0;
        for (step, size) in steps {
            if step == "audio" { ai += 1; }
            let n = if size == 0 { 3 } else { 18 };
            for k in 0..=n {
                if wait(paused, cancel) {
                    emit(Event::ItemCancelled { index, stage: Stage::Download });
                    return finish(sink, saved, failed, total, bytes, started, true);
                }
                let pct = k as f64 / n as f64 * 100.0;
                let mut p = Progress { index, stage: Some(Stage::Download), step: Some(step.into()), pct: Some(pct), ..Default::default() };
                if step == "subs" {
                    p.note = Some("subtitles…".into());
                } else if size > 0 {
                    p.done = Some((size as f64 * pct / 100.0) as u64);
                    p.total = Some(size);
                    p.speed = Some(14_500_000.0 + (k as f64 * 0.7).sin() * 2_000_000.0);
                    p.eta = Some((n - k) as f64 * 1.3);
                }
                if step == "audio" && audio_n > 1 {
                    p.audio_i = Some(ai);
                    p.audio_n = Some(audio_n);
                }
                emit(Event::Progress(p));
            }
        }
        emit(Event::StageDone { index, stage: Stage::Download, size: 431_000_000, elapsed: 21.0, merged: !audio_only });
        emit(Event::TcStart { index });
        log("$ ffmpeg -i video.mkv -c:v h264_nvenc -cq 16 -preset p6 -c:a aac -b:a 320k out.mp4", LogLevel::Command);
        let frames = 112_440u64;
        for k in 0..=24u64 {
            if wait(paused, cancel) {
                emit(Event::ItemCancelled { index, stage: Stage::Transcode });
                return finish(sink, saved, failed, total, bytes, started, true);
            }
            let pct = k as f64 / 24.0 * 100.0;
            if fail && pct >= 40.0 {
                let msg = "ffmpeg couldn't process the file";
                log("Error while decoding stream #0:0: Invalid data found when processing input", LogLevel::Error);
                emit(Event::ItemError { index, total, message: msg.into(), stage: Stage::Transcode });
                failed += 1;
                break;
            }
            emit(Event::Progress(Progress {
                index, stage: Some(Stage::Transcode), pct: Some(pct),
                frame: Some(frames * k / 24), frames: Some(frames), speed: Some(3.4), eta: Some((24 - k) as f64 * 1.6),
                ..Default::default()
            }));
        }
        if fail {
            continue;
        }
        emit(Event::StageDone { index, stage: Stage::Transcode, size: 318_400_000, elapsed: 47.0, merged: false });
        let folder = if it.folder.is_empty() { s.out_dir.clone() } else { format!("{}\\{}", s.out_dir, it.folder) };
        let ext = if audio_only { s.codec.clone() } else if s.subs != "off" { "mkv".into() } else { "mp4".into() };
        let path = format!("{folder}\\{}How the James Webb Space Telescope sees the universe.{ext}", it.prefix);
        let srt = if s.subs == "embed_srt" && !audio_only { vec![path.rsplit_once('.').unwrap().0.to_string() + ".en.srt", path.rsplit_once('.').unwrap().0.to_string() + ".ru.srt"] } else { vec![] };
        log(&format!("Saved: {path}"), LogLevel::Ok);
        emit(Event::ItemDone { index, path, size: 318_400_000, srt });
        saved += 1;
        bytes += 318_400_000;
    }
    finish(sink, saved, failed, total, bytes, started, false)
}

fn finish(sink: &EventSink, saved: usize, failed: usize, total: usize, bytes: u64, started: std::time::Instant, cancelled: bool) {
    sink(Event::Finished { saved, failed, total, bytes, elapsed: started.elapsed().as_secs_f64(), cancelled });
}
