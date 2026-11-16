//! Очередь загрузок (Job в Python). Скачивание и перекодирование идут в разных потоках: пока ffmpeg
//! перекодирует одно видео, следующее уже скачивается (сеть и процессор не ждут друг друга).
use crate::encode::{self, Gpu, audio_out};
use crate::errors::{friendly_error, http403, login_re, outdated_re};
use crate::media::{AudioRule, Track, audio_tracks, is_audio_only, official_subs, pick_audio, pick_subs, select_video, video_formats};
use crate::net::net_args;
use crate::preview::{INFO_CACHE_TTL, ThumbFetch, preview_data};
use crate::procs;
use crate::util::{Trim, base_lang, iso3, move_file, num_str, quality_tag, stem_suffix, trim_duration, trim_in, trim_out, trim_tag, unique_path};
use crate::{Event, EventSink, LogLevel, Progress, Settings, Shared, Stage, i18n};
use regex::Regex;
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader, Read};
use std::os::windows::io::AsRawHandle;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Condvar, Mutex, OnceLock, mpsc};
use std::time::{Duration, Instant};

/// Шаблоны имени файла для yt-dlp (к имени потом добавляется «[1080p h264]»).
pub fn name_tpl(id: &str) -> &'static str {
    match id {
        "channel_title" => "%(uploader,channel,creator|)s — %(title).150B",
        "date_title" => "%(upload_date>%Y-%m-%d|)s %(title).160B",
        "title_id" => "%(title).150B [%(id)s]",
        _ => "%(title).170B",
    }
}

pub const SPONSOR_CATS: &str = "sponsor,selfpromo,interaction";

#[derive(Debug)]
pub(crate) enum JobError {
    Cancelled,
    /// msg — понятный текст; raw — строки ERROR yt-dlp.
    Download { msg: String, raw: String },
    /// ffmpeg не справился (RuntimeError в Python — после него пробуем процессор вместо видеокарты).
    Ffmpeg(String),
    Other(String),
}

impl std::fmt::Display for JobError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            JobError::Cancelled => Ok(()),
            JobError::Download { msg, .. } => f.write_str(msg),
            JobError::Ffmpeg(m) | JobError::Other(m) => f.write_str(m),
        }
    }
}

impl From<std::io::Error> for JobError {
    fn from(e: std::io::Error) -> Self { JobError::Other(e.to_string()) }
}

type R<T> = Result<T, JobError>;

/// Одна ссылка в очереди (для плейлиста — одно его видео).
pub(crate) struct Task {
    pub index: usize,
    pub url: String,
    pub folder: String,
    pub prefix: String,
    pub trim: Option<Trim>,
    pub audio: Option<AudioRule>,
    tmp: Option<tempfile::TempDir>,
    info_path: Option<PathBuf>,
    audio_mode: bool,
    srt_saved: Vec<String>,
    /// повтор со входом из браузера (cookies от расширения)
    browser: bool,
}

impl Task {
    pub fn new(index: usize, url: String, folder: String, prefix: String, trim: Option<Trim>, audio: Option<AudioRule>) -> Task {
        Task { index, url, folder, prefix, trim, audio, tmp: None, info_path: None, audio_mode: false, srt_saved: vec![], browser: false }
    }
    fn tmp(&self) -> &Path { self.tmp.as_ref().map(|t| t.path()).expect("task tmp") }
    fn cleanup(&mut self) { self.tmp = None; }
}

enum Prep {
    Video { src: PathBuf, subs: Vec<(String, PathBuf)>, chosen: Vec<Track>, tracks: Vec<Track>, video_copy: bool },
    Audio { src: PathBuf, lang: Option<String>, title: String, artist: String },
}

// ---------------------------------------------------------------- pause / cancel
struct CtlState {
    paused: bool,
    /// "dl" / "tc" -> (pid, handle процесса из Child)
    procs: HashMap<&'static str, (u32, usize)>,
}

/// Управление идущей очередью: пауза, отмена, запущенные процессы.
pub struct Ctl {
    sink: EventSink,
    cancelled: AtomicBool,
    st: Mutex<CtlState>,
    resumed: Condvar,
}

impl Ctl {
    pub fn new(sink: EventSink) -> Ctl {
        Ctl { sink, cancelled: AtomicBool::new(false), st: Mutex::new(CtlState { paused: false, procs: HashMap::new() }), resumed: Condvar::new() }
    }

    pub fn is_cancelled(&self) -> bool { self.cancelled.load(Ordering::SeqCst) }

    pub fn is_paused(&self) -> bool { self.st.lock().unwrap().paused }

    /// PID запущенных сейчас yt-dlp / ffmpeg (корни деревьев) — для тестов и диагностики.
    pub fn pids(&self) -> Vec<u32> { self.st.lock().unwrap().procs.values().map(|p| p.0).collect() }

    pub fn set_paused(&self, on: bool) {
        {
            let mut g = self.st.lock().unwrap();
            if on == g.paused || self.is_cancelled() {
                return;
            }
            g.paused = on;
            if !on {
                self.resumed.notify_all();
            }
            for &(pid, h) in g.procs.values() {
                if procs::alive(h) {
                    procs::suspend_tree(pid, on);
                }
            }
        }
        (self.sink)(Event::Paused { on });
    }

    fn wait_resumed(&self) -> R<()> {
        let mut g = self.st.lock().unwrap();
        while g.paused && !self.is_cancelled() {
            g = self.resumed.wait_timeout(g, Duration::from_millis(250)).unwrap().0;
        }
        drop(g);
        if self.is_cancelled() { Err(JobError::Cancelled) } else { Ok(()) }
    }

    pub fn cancel(&self) {
        self.cancelled.store(true, Ordering::SeqCst);
        self.resumed.notify_all();
        let list: Vec<(u32, usize)> = self.st.lock().unwrap().procs.values().copied().collect();
        for (pid, h) in list {
            if procs::alive(h) {
                procs::kill_tree(pid);
            }
        }
    }

    /// Запуск процесса: ждёт снятия паузы; если пауза включилась в этот момент — сразу замораживает.
    fn spawn(&self, slot: &'static str, mut cmd: Command) -> R<Child> {
        self.wait_resumed()?;
        let child = cmd.spawn();
        drop(cmd); // закрыть свои копии концов каналов, иначе чтение не закончится
        let child = child?;
        {
            let mut g = self.st.lock().unwrap();
            g.procs.insert(slot, (child.id(), child.as_raw_handle() as usize));
            if g.paused {
                procs::suspend_tree(child.id(), true);
            }
        }
        if self.is_cancelled() {
            self.cancel();
        }
        Ok(child)
    }

    fn unregister(&self, slot: &'static str) { self.st.lock().unwrap().procs.remove(slot); }
}

struct Sem {
    n: Mutex<usize>,
    cv: Condvar,
}

impl Sem {
    fn acquire(&self, timeout: Duration) -> bool {
        let g = self.n.lock().unwrap();
        let (mut g, _) = self.cv.wait_timeout_while(g, timeout, |n| *n == 0).unwrap();
        if *g == 0 { return false; }
        *g -= 1;
        true
    }
    fn release(&self) {
        *self.n.lock().unwrap() += 1;
        self.cv.notify_one();
    }
}

#[derive(Default)]
struct Stats { saved: usize, bytes: u64, failed: usize }

/// Длительность, высота, fps, кадры, видеокодек, аудиокодеки (probe в Python).
#[derive(Debug, Default, Clone)]
pub struct Probe {
    pub duration: Option<f64>,
    pub height: Option<u32>,
    pub fps: f64,
    pub frames: Option<u64>,
    pub vcodec: Option<String>,
    pub acodecs: Vec<String>,
}

pub fn probe(path: &Path) -> Probe {
    let p = path.to_string_lossy().into_owned();
    let run = |a: &[&str]| {
        let mut v: Vec<String> = a.iter().map(|s| s.to_string()).collect();
        v.push(p.clone());
        procs::run_quiet(&v, Duration::from_secs(30))
    };
    let Ok(r) = run(&["ffprobe", "-v", "error", "-select_streams", "v:0", "-show_entries",
        "stream=codec_name,height,r_frame_rate,nb_frames:format=duration", "-of", "json"]) else { return Probe::default() };
    let Ok(j) = serde_json::from_str::<Value>(&String::from_utf8_lossy(&r.stdout)) else { return Probe::default() };
    let s = j.get("streams").and_then(Value::as_array).and_then(|a| a.first()).cloned().unwrap_or(Value::Null);
    let duration = j.get("format").and_then(|f| crate::util::num(f.get("duration"))).filter(|d| *d != 0.0);
    let rate = s.get("r_frame_rate").and_then(Value::as_str).unwrap_or("0/1");
    let (n, d) = rate.split_once('/').unwrap_or((rate, "1"));
    let (n, d) = (num_str(n).unwrap_or(0.0), num_str(if d.is_empty() { "1" } else { d }).unwrap_or(1.0));
    let fps = if d != 0.0 { n / d } else { 0.0 };
    let acodecs = run(&["ffprobe", "-v", "error", "-select_streams", "a", "-show_entries", "stream=codec_name", "-of", "csv=p=0"])
        .map(|o| String::from_utf8_lossy(&o.stdout).split_whitespace().map(str::to_string).collect())
        .unwrap_or_default();
    Probe {
        duration,
        height: crate::util::num(s.get("height")).map(|h| h as u32),
        fps,
        frames: match duration { Some(d) if fps != 0.0 => Some((d * fps) as u64), _ => None },
        vcodec: s.get("codec_name").and_then(Value::as_str).map(str::to_string),
        acodecs,
    }
}

fn sv(a: &[&str]) -> Vec<String> { a.iter().map(|s| s.to_string()).collect() }

fn cmdline(args: &[String]) -> String {
    args.iter().map(|a| if a.contains(' ') { format!("\"{a}\"") } else { a.clone() }).collect::<Vec<_>>().join(" ")
}

pub(crate) struct Job {
    sh: Arc<Shared>,
    pub ctl: Arc<Ctl>,
    opts: Settings,
    lang: String,
    total: usize,
    last_emit: Mutex<[Option<Instant>; 3]>,
    stats: Mutex<Stats>,
}

impl Job {
    pub fn new(sh: Arc<Shared>, ctl: Arc<Ctl>, opts: Settings, total: usize) -> Job {
        let lang = i18n::resolve_lang(&opts.lang);
        Job { sh, ctl, opts, lang, total, last_emit: Mutex::new([None; 3]), stats: Mutex::default() }
    }

    fn emit(&self, ev: Event) { (self.sh.sink)(ev) }

    fn emit_progress(&self, force: bool, p: Progress) {
        let k = match p.stage { None => 0, Some(Stage::Download) => 1, Some(Stage::Transcode) => 2 };
        let now = Instant::now();
        {
            let mut g = self.last_emit.lock().unwrap();
            if !force && g[k].is_some_and(|t| now.duration_since(t) < Duration::from_millis(400)) {
                return;
            }
            g[k] = Some(now);
        }
        self.emit(Event::Progress(p));
    }

    fn t(&self, key: &str, p: &[(&str, &str)]) -> String { i18n::tr(&self.lang, key, p) }

    fn log(&self, line: impl Into<String>, level: LogLevel) { self.emit(Event::Log { line: line.into(), level }) }

    fn dl_progress(&self, idx: usize) -> Progress { Progress { index: idx, stage: Some(Stage::Download), ..Default::default() } }

    // ---- процессы
    /// Запустить, читать объединённые stdout+stderr построчно, дождаться. Код выхода.
    fn run_proc(&self, args: &[String], mut on_line: impl FnMut(&str), slot: &'static str) -> R<i32> {
        self.log(format!("$ {}", cmdline(args)), LogLevel::Command);
        let (reader, writer) = std::io::pipe()?;
        let mut cmd = procs::command(args, procs::NO_WINDOW | procs::BELOW_NORMAL);
        cmd.stdout(writer.try_clone()?).stderr(writer);
        let mut child = self.ctl.spawn(slot, cmd)?;
        let mut r = BufReader::new(reader);
        let mut buf = vec![];
        loop {
            buf.clear();
            match r.read_until(b'\n', &mut buf) {
                Ok(0) | Err(_) => break,
                Ok(_) => {
                    let line = String::from_utf8_lossy(&buf);
                    let line = line.trim_end();
                    if !line.is_empty() {
                        on_line(line);
                    }
                }
            }
        }
        let code = child.wait();
        self.ctl.unregister(slot);
        drop(child);
        if self.ctl.is_cancelled() {
            return Err(JobError::Cancelled);
        }
        Ok(code?.code().unwrap_or(-1))
    }

    fn net(&self, task: &Task) -> R<Vec<String>> {
        let bc = if task.browser { self.sh.cookies.for_url(&task.url) } else { None };
        net_args(&self.opts, Some(task.tmp()), bc.as_deref())
            .map_err(|_| JobError::Download { msg: self.t("py.err.cookieNoFile", &[]), raw: "cookies file missing".into() })
    }

    // ---- очередь
    pub fn run(self: Arc<Self>, tasks: Vec<Task>) {
        let t0 = Instant::now();
        let up = &self.sh.updater;
        if !up.is_idle() {
            self.log(self.t("py.waitUpdate", &[]), LogLevel::Normal);
            up.wait_idle(Duration::from_secs(300));
        }
        if self.opts.parallel == "on" && tasks.len() > 1 {
            self.run_pipeline(tasks);
        } else {
            for mut task in tasks {
                if self.ctl.is_cancelled() {
                    break;
                }
                if let Some(prep) = self.download_stage(&mut task) {
                    self.finish_stage(&mut task, prep);
                }
            }
        }
        let s = self.stats.lock().unwrap();
        self.emit(Event::Finished {
            saved: s.saved, failed: s.failed, total: self.total, bytes: s.bytes,
            elapsed: t0.elapsed().as_secs_f64(), cancelled: self.ctl.is_cancelled(),
        });
    }

    fn run_pipeline(self: &Arc<Self>, tasks: Vec<Task>) {
        let (tx, rx) = mpsc::channel::<(Task, Prep)>();
        // не больше одного скачанного видео «в запасе»: временные файлы не копятся на диске
        let slots = Arc::new(Sem { n: Mutex::new(2), cv: Condvar::new() });
        let me = self.clone();
        let sl = slots.clone();
        let th = std::thread::spawn(move || {
            'outer: for mut task in tasks {
                while !sl.acquire(Duration::from_millis(250)) {
                    if me.ctl.is_cancelled() {
                        break 'outer;
                    }
                }
                if me.ctl.is_cancelled() {
                    sl.release();
                    break;
                }
                match me.download_stage(&mut task) {
                    None => sl.release(),
                    Some(prep) => {
                        if tx.send((task, prep)).is_err() {
                            break;
                        }
                    }
                }
            }
        });
        for (mut task, prep) in rx {
            self.finish_stage(&mut task, prep);
            slots.release();
        }
        let _ = th.join();
    }

    fn count_failed(&self) { self.stats.lock().unwrap().failed += 1; }

    fn error_message(&self, e: &JobError) -> String {
        let m = e.to_string();
        if m.is_empty() { self.t("py.err.unknown", &[]) } else { m }
    }

    /// Получить данные и скачать. None — ошибка или отмена (о них уже сообщено).
    fn download_stage(&self, task: &mut Task) -> Option<Prep> {
        self.emit(Event::Item { index: task.index, total: self.total });
        match self.attempt(task) {
            Ok(p) => return Some(p),
            Err(JobError::Cancelled) => self.emit(Event::ItemCancelled { index: task.index, stage: Stage::Download }),
            Err(e) => {
                self.count_failed();
                let msg = self.error_message(&e);
                self.log(msg.clone(), LogLevel::Error);
                self.emit(Event::ItemError { index: task.index, total: self.total, message: msg, stage: Stage::Download });
            }
        }
        task.cleanup();
        None
    }

    fn finish_stage(&self, task: &mut Task, prep: Prep) {
        let res = (|| -> R<PathBuf> {
            if self.ctl.is_cancelled() {
                return Err(JobError::Cancelled);
            }
            self.emit(Event::TcStart { index: task.index });
            self.finalize(task, prep)
        })();
        match res.and_then(|p| Ok((std::fs::metadata(&p)?.len(), p))) {
            Ok((size, path)) => {
                {
                    let mut s = self.stats.lock().unwrap();
                    s.saved += 1;
                    s.bytes += size;
                }
                self.emit(Event::ItemDone { index: task.index, path: path.to_string_lossy().into_owned(), size, srt: task.srt_saved.clone() });
            }
            Err(JobError::Cancelled) => self.emit(Event::ItemCancelled { index: task.index, stage: Stage::Transcode }),
            Err(e) => {
                self.count_failed();
                let msg = self.error_message(&e);
                self.log(msg.clone(), LogLevel::Error);
                self.emit(Event::ItemError { index: task.index, total: self.total, message: msg, stage: Stage::Transcode });
            }
        }
        task.cleanup();
    }

    fn drop_cache(&self, url: &str) { self.sh.info_cache.lock().unwrap().remove(url); }

    fn attempt(&self, task: &mut Task) -> R<Prep> {
        match self.attempt_inner(task) {
            Err(JobError::Download { msg, raw }) => {
                // сайт требует вход, а в параметрах он не настроен — пробуем cookies, присланные расширением
                if task.browser || self.opts.cookies != "off" || !login_re().is_match(&raw)
                    || self.sh.cookies.for_url(&task.url).is_none()
                {
                    return Err(JobError::Download { msg, raw });
                }
                self.log(self.t("py.browserLogin", &[]), LogLevel::Normal);
                task.cleanup();
                task.browser = true;
                self.drop_cache(&task.url);
                self.attempt_inner(task)
            }
            r => r,
        }
    }

    fn attempt_inner(&self, task: &mut Task) -> R<Prep> {
        let mut e = match self.prepare(task) {
            Err(e @ JobError::Download { .. }) => e,
            r => return r,
        };
        if let JobError::Download { raw, .. } = &e {
            if http403(raw) {
                // TikTok и другие иногда отказывают при частых запросах — ждём и пробуем ещё раз с нуля
                self.log(self.t("py.retry403", &[]), LogLevel::Normal);
                task.cleanup();
                self.drop_cache(&task.url);
                for _ in 0..8 {
                    if self.ctl.is_cancelled() {
                        return Err(JobError::Cancelled);
                    }
                    std::thread::sleep(Duration::from_millis(500));
                }
                match self.prepare(task) {
                    Err(e2 @ JobError::Download { .. }) => e = e2,
                    r => return r,
                }
            }
        }
        // частая причина падений — устаревший yt-dlp: обновляем и пробуем ещё раз
        let JobError::Download { raw, .. } = &e else { return Err(e) };
        if !(self.sh.updater.enabled() && outdated_re().is_match(raw)) {
            return Err(e);
        }
        self.log(self.t("py.outdated", &[]), LogLevel::Normal);
        task.cleanup();
        if !self.sh.updater.check(&self.sh, true) {
            return Err(e);
        }
        self.prepare(task)
    }

    // ---- download half
    fn prepare(&self, task: &mut Task) -> R<Prep> {
        let o = &self.opts;
        task.tmp = Some(tempfile::Builder::new().prefix("topicture_").tempdir()?);
        self.net(task)?;
        task.audio_mode = false;
        let meta = self.fetch_info(task)?;
        if !task.folder.is_empty() {
            // видео из плейлиста — превью показываем по уже полученным данным
            let fx = ThumbFetch { proxy: o.proxy.clone(), timeout: Duration::from_secs(5) };
            let data = preview_data(&meta, &task.url, Some(&fx));
            self.emit(Event::Meta { index: task.index, data });
        }
        if audio_out(&o.codec).is_some() || is_audio_only(&meta) {
            return self.prepare_audio(task, &meta);
        }
        // видео: SDR вместо HDR; H.264 от YouTube — без перекодирования (кроме фрагмента: его режем точно по кадру)
        let (vpick, video_copy) = select_video(&video_formats(&meta), &o.max_height, &o.codec, task.trim.is_none());
        if let Some(v) = &vpick {
            let q = format!("{}p{}", v.h, v.fps);
            if video_copy {
                self.log(self.t("py.copyVideo", &[("q", &q)]), LogLevel::Normal);
            } else if o.codec != "copy" {
                self.log(self.t("py.videoPick", &[("q", &q), ("vc", &v.vc), ("hdr", if v.hdr { " (HDR)" } else { "" })]), LogLevel::Normal);
            }
        }
        // озвучка: правило из настроек или выбор для конкретного видео
        let tracks = audio_tracks(&meta, video_copy);
        let rule = task.audio.clone().unwrap_or_else(|| AudioRule::Rule(o.audio_lang.clone()));
        let chosen = if tracks.len() > 1 { pick_audio(&tracks, &rule, &self.lang) } else { vec![] };
        if !chosen.is_empty() {
            let list = chosen.iter().map(|t| format!("{}{}", t.lang, if t.original { self.t("py.orig", &[]) } else { String::new() }))
                .collect::<Vec<_>>().join(", ");
            self.log(self.t("py.tracks", &[("n", &chosen.len().to_string()), ("total", &tracks.len().to_string()), ("list", &list)]), LogLevel::Normal);
        }
        // субтитры: только официальные, автоматические не берём
        let mut subs_keys = vec![];
        if o.subs != "off" {
            let avail = official_subs(&meta);
            let audio_langs: Vec<String> = chosen.iter().map(|t| t.lang.clone()).collect();
            subs_keys = pick_subs(&avail, &o.subs_lang, &audio_langs, meta.get("language").and_then(Value::as_str), &self.lang);
            if !subs_keys.is_empty() {
                self.log(self.t("py.subs", &[("list", &subs_keys.join(", "))]), LogLevel::Normal);
            } else if !avail.is_empty() {
                self.log(self.t("py.subsMissing", &[("list", &avail.join(", "))]), LogLevel::Normal);
            } else {
                self.log(self.t("py.subsNone", &[]), LogLevel::Normal);
            }
        }
        // план загрузки — чтобы прогресс показывал только реальные шаги
        self.emit(Event::Plan { index: task.index, subs: !subs_keys.is_empty(), audio_n: chosen.len().max(1), copy: video_copy, audio_only: false });
        let (src, subs) = self.download(task, &chosen, &subs_keys, vpick.as_ref().map(|v| v.id.as_str()), None)?;
        Ok(Prep::Video { src, subs, chosen, tracks, video_copy })
    }

    fn prepare_audio(&self, task: &mut Task, meta: &Value) -> R<Prep> {
        let o = &self.opts;
        let source_audio = is_audio_only(meta);
        self.log(if source_audio { self.t("py.audioOnly", &[]) } else { self.t("py.audioOut", &[("f", &o.codec.to_uppercase())]) }, LogLevel::Normal);
        task.audio_mode = true;
        let tracks = audio_tracks(meta, false);
        let rule = task.audio.clone().unwrap_or_else(|| AudioRule::Rule(o.audio_lang.clone()));
        // в аудиофайле одна дорожка — первая из выбранных (оригинал, если он отмечен)
        let chosen: Vec<Track> = if tracks.len() > 1 { pick_audio(&tracks, &rule, &self.lang).into_iter().take(1).collect() } else { vec![] };
        self.emit(Event::Plan { index: task.index, subs: false, audio_n: 1, copy: false, audio_only: true });
        let fmt = format!("{}ba[acodec^=mp4a]/ba[acodec=mp3]/ba/b", chosen.first().map(|t| format!("{}/", t.id)).unwrap_or_default());
        let (src, _) = self.download(task, &[], &[], None, Some(fmt))?;
        let lang = chosen.first().or(tracks.first()).map(|t| t.lang.clone());
        let js = |k: &str| meta.get(k).and_then(Value::as_str).filter(|s| !s.is_empty());
        Ok(Prep::Audio {
            src, lang,
            title: js("title").unwrap_or("").to_string(),
            artist: js("uploader").or(js("channel")).unwrap_or("").to_string(),
        })
    }

    /// yt-dlp -J во временный файл; загрузка потом идёт по нему (--load-info-json), чтобы не ходить на сайт дважды.
    fn fetch_info(&self, task: &mut Task) -> R<Value> {
        self.emit_progress(true, Progress { step: Some("video".into()), note: Some(self.t("py.fetching", &[])), ..self.dl_progress(task.index) });
        let path = task.tmp().join("info.json");
        // данные уже получены для превью — используем их: меньше запросов к сайту (TikTok за частые даёт 403)
        let cached = {
            let c = self.sh.info_cache.lock().unwrap();
            c.get(&task.url).filter(|e| e.at.elapsed() < INFO_CACHE_TTL).map(|e| (e.json.clone(), e.browser))
        };
        if let Some((json, browser)) = cached {
            if browser {
                task.browser = true; // превью получено со входом из браузера — и качаем так же
            }
            std::fs::write(&path, &json)?;
            task.info_path = Some(path);
            return serde_json::from_str(&String::from_utf8_lossy(&json)).map_err(|e| JobError::Other(e.to_string()));
        }
        let mut args = sv(&["yt-dlp", "-J", "--no-playlist", "--no-warnings"]);
        args.extend(self.net(task)?);
        args.push("--".into());
        args.push(task.url.clone());
        self.log(format!("$ {}", args.join(" ")), LogLevel::Command);
        let out = std::fs::File::create(&path)?;
        let mut cmd = procs::command(&args, procs::NO_WINDOW | procs::BELOW_NORMAL);
        cmd.stdout(out).stderr(Stdio::piped());
        let mut child = self.ctl.spawn("dl", cmd)?;
        let mut err = vec![];
        if let Some(mut se) = child.stderr.take() {
            let _ = se.read_to_end(&mut err);
        }
        let code = child.wait();
        self.ctl.unregister("dl");
        drop(child);
        if self.ctl.is_cancelled() {
            return Err(JobError::Cancelled);
        }
        let code = code?.code().unwrap_or(-1);
        let err = String::from_utf8_lossy(&err);
        let errors: Vec<&str> = err.lines().filter(|l| l.starts_with("ERROR")).collect();
        for l in err.lines() {
            if !l.trim().is_empty() {
                self.log(l, if l.starts_with("ERROR") { LogLevel::Error } else { LogLevel::Normal });
            }
        }
        if code != 0 {
            let msg = match errors.last() {
                Some(e) => friendly_error(e, &self.lang),
                None => self.t("py.ytdlpExit", &[("code", &code.to_string())]),
            };
            return Err(JobError::Download { msg, raw: errors.join("\n") });
        }
        task.info_path = Some(path.clone());
        let text = std::fs::read(&path)?;
        serde_json::from_str(&String::from_utf8_lossy(&text)).map_err(|e| JobError::Other(e.to_string()))
    }

    fn download(&self, task: &mut Task, chosen: &[Track], subs_keys: &[String], video_id: Option<&str>, fmt: Option<String>)
        -> R<(PathBuf, Vec<(String, PathBuf)>)> {
        let o = &self.opts;
        let lim = match o.max_height.trim().parse::<u32>() {
            Ok(h) if o.max_height != "best" => format!("[height<={h}]"),
            _ => String::new(),
        };
        let v = video_id.map(str::to_string).unwrap_or_else(|| format!("bv*{lim}"));
        // запасные варианты: не все сайты сообщают размер кадра, у некоторых видео и звук — один файл
        let tail = format!("bv*{lim}+ba/b{lim}/bv*+ba/b");
        let fmt = match fmt {
            Some(f) => f,
            None if !chosen.is_empty() => {
                format!("{v}+{}/{v}+ba/{tail}", chosen.iter().map(|t| t.id.as_str()).collect::<Vec<_>>().join("+"))
            }
            None if video_id.is_some() => format!("{v}+ba[acodec^=mp4a]/{v}+ba/{v}/{tail}"),
            None => tail.clone(),
        };
        let name = name_tpl(&o.name_tpl);
        let mut args = sv(&["yt-dlp", "--load-info-json"]);
        args.push(task.info_path.as_ref().map(|p| p.to_string_lossy().into_owned()).unwrap_or_default());
        args.push("-f".into());
        args.push(fmt);
        args.extend(self.net(task)?);
        args.extend(sv(&["--merge-output-format", "mkv", "--newline", "--color", "never", "--progress-template",
            "download:PROG %(progress.downloaded_bytes)s %(progress.total_bytes)s %(progress.total_bytes_estimate)s %(progress.speed)s %(progress.eta)s",
            "-o"]));
        args.push(task.tmp().join(format!("{name}.%(ext)s")).to_string_lossy().into_owned());
        if crate::settings::RATE_LIMITS[1..].contains(&o.rate_limit.as_str()) {
            args.extend(["-r".into(), o.rate_limit.clone()]);
        }
        // SponsorBlock (только YouTube): с фрагментом не сочетается — сдвинулось бы время
        if o.sponsorblock == "remove" && task.trim.is_none() {
            args.extend(sv(&["--sponsorblock-remove", SPONSOR_CATS]));
        } else if o.sponsorblock == "mark" {
            args.extend(sv(&["--sponsorblock-mark", SPONSOR_CATS, "--embed-chapters"]));
        }
        if chosen.len() > 1 {
            args.push("--audio-multistreams".into());
        }
        if !subs_keys.is_empty() {
            args.extend(["--write-subs".into(), "--sub-langs".into(), subs_keys.join(","), "--convert-subs".into(), "srt".into()]);
        }
        let t0 = Instant::now();
        // step: None — субтитры (их прогресс не показываем) или ещё ничего не началось
        let mut step: Option<&'static str> = None;
        let mut errors: Vec<String> = vec![];
        let mut media = 0usize;
        let n_audio = chosen.len().max(1);
        let idx = task.index;
        let audio_mode = task.audio_mode;
        static SUB_RE: OnceLock<Regex> = OnceLock::new();
        let sub_re = SUB_RE.get_or_init(|| Regex::new(r"(?i)\.(vtt|srt|ttml|srv\d|json3)$").unwrap());
        let code = self.run_proc(&args, |line| {
            if let Some(rest) = line.strip_prefix("PROG ") {
                let Some(st) = step else { return };
                let p: Vec<Option<f64>> = rest.split_whitespace().take(5).map(num_str).collect();
                let g = |i: usize| p.get(i).copied().flatten();
                let (done, tot, est, speed, eta) = (g(0), g(1), g(2), g(3), g(4));
                let tot = crate::util::truthy(tot).or(est);
                let pct = match (done, tot) { (Some(d), Some(t)) if t != 0.0 => Some(d / t * 100.0), _ => None };
                self.emit_progress(false, Progress {
                    step: Some(st.into()), done: done.map(|d| d as u64), total: tot.map(|t| t as u64), speed, eta, pct,
                    audio_i: Some(media.saturating_sub(1).max(1)), audio_n: Some(n_audio), ..self.dl_progress(idx)
                });
                return;
            }
            self.log(line, if line.starts_with("ERROR") { LogLevel::Error } else { LogLevel::Normal });
            if line.starts_with("ERROR") {
                errors.push(line.to_string());
            } else if line.contains("[download] Destination:") {
                if sub_re.is_match(line) {
                    step = None;
                    self.emit_progress(true, Progress { step: Some("subs".into()), note: Some(self.t("py.subsNote", &[])), ..self.dl_progress(idx) });
                } else {
                    media += 1;
                    let st = if audio_mode || media > 1 { "audio" } else { "video" };
                    step = Some(st);
                    self.emit_progress(true, Progress {
                        step: Some(st.into()), pct: Some(0.0), audio_i: Some(media.saturating_sub(1).max(1)), audio_n: Some(n_audio),
                        ..self.dl_progress(idx)
                    });
                }
            } else if line.starts_with("[Merger]") {
                self.emit_progress(true, Progress { step: Some("merge".into()), pct: Some(100.0), ..self.dl_progress(idx) });
            } else if line.starts_with("[SponsorBlock]") || line.starts_with("[ModifyChapters]") {
                self.emit_progress(true, Progress {
                    step: Some("merge".into()), pct: Some(100.0), note: Some(self.t("py.sponsorNote", &[])), ..self.dl_progress(idx)
                });
            }
        }, "dl")?;
        const MEDIA: &[&str] = &[".mkv", ".webm", ".mp4", ".mov", ".m4a", ".mp3", ".opus", ".ogg", ".aac", ".flac", ".wav", ".flv", ".ts"];
        let mut files: Vec<(PathBuf, u64)> = vec![];
        let mut srts: Vec<PathBuf> = vec![];
        for e in std::fs::read_dir(task.tmp())?.flatten() {
            let p = e.path();
            let Ok(md) = e.metadata() else { continue };
            if !md.is_file() {
                continue;
            }
            let name = e.file_name().to_string_lossy().to_lowercase();
            if name.ends_with(".srt") {
                srts.push(p.clone());
            }
            if name.ends_with(".part") || name.ends_with(".ytdl") {
                continue;
            }
            let suf = stem_suffix(&p).1.to_lowercase();
            if MEDIA.contains(&suf.as_str()) {
                files.push((p, md.len()));
            }
        }
        if code != 0 || files.is_empty() {
            if errors.is_empty() && code == 0 {
                // yt-dlp отработал, но видео в публикации нет (например, картинки)
                return Err(JobError::Download { msg: self.t("py.err.noVideo", &[]), raw: "no video".into() });
            }
            let msg = match errors.last() {
                Some(e) => friendly_error(e, &self.lang),
                None => self.t("py.ytdlpExit", &[("code", &code.to_string())]),
            };
            return Err(JobError::Download { msg, raw: errors.join("\n") });
        }
        let (src, size) = crate::media::max_first(files, |f| f.1).unwrap();
        srts.sort();
        let mut subs = vec![];
        for f in srts {
            // «Название.en.srt» -> en
            let (stem, _) = stem_suffix(&f);
            let size = std::fs::metadata(&f).map(|m| m.len()).unwrap_or(0);
            if let Some((_, lang)) = stem.rsplit_once('.') {
                if size > 0 {
                    subs.push((lang.to_string(), f));
                }
            }
        }
        self.emit(Event::StageDone { index: idx, stage: Stage::Download, size, elapsed: t0.elapsed().as_secs_f64(), merged: media > 1 });
        Ok((src, subs))
    }

    // ---- transcode half
    fn out_dir(&self, task: &Task) -> R<PathBuf> {
        let mut d = PathBuf::from(&self.opts.out_dir);
        if !task.folder.is_empty() {
            d = d.join(&task.folder);
        }
        std::fs::create_dir_all(&d)?;
        Ok(d)
    }

    /// «Канал — Название» без канала, «Дата Название» без даты: убираем висящие разделители.
    fn stem(&self, task: &Task, src: &Path) -> String {
        static R: OnceLock<Regex> = OnceLock::new();
        let r = R.get_or_init(|| Regex::new(r"^[\s—–-]+").unwrap());
        let (stem, _) = stem_suffix(src);
        let clean = r.replace(&stem, "").trim().to_string();
        format!("{}{}", task.prefix, if clean.is_empty() { stem } else { clean })
    }

    fn finalize(&self, task: &mut Task, p: Prep) -> R<PathBuf> {
        let (src, sub_files, chosen, tracks, video_copy) = match p {
            Prep::Audio { src, lang, title, artist } => return self.finalize_audio(task, src, lang, title, artist),
            Prep::Video { src, subs, chosen, tracks, video_copy } => (src, subs, chosen, tracks, video_copy),
        };
        let o = &self.opts;
        let out_dir = self.out_dir(task)?;
        let info = probe(&src);
        let c = encode::codec(&o.codec).unwrap_or_else(|| encode::codec("h264").unwrap());
        let stem = self.stem(task, &src);
        let q = quality_tag(info.height, info.fps);
        let a_tag = if chosen.len() > 3 {
            " multi".to_string()
        } else if chosen.len() > 1 || chosen.first().is_some_and(|t| !t.original) {
            format!(" {}", chosen.iter().map(|t| base_lang(&t.lang)).collect::<Vec<_>>().join("+"))
        } else {
            String::new()
        };
        let audio_meta: Vec<(String, String)> = if !chosen.is_empty() {
            chosen.iter().map(|t| {
                let title = if t.original { self.t("py.trackOriginal", &[]) } else if t.auto { self.t("py.trackAuto", &[]) } else { String::new() };
                (t.lang.clone(), title)
            }).collect()
        } else {
            // одна дорожка — всё равно подпишем язык, если сайт его знает
            tracks.first().filter(|t| t.lang != "und").map(|t| vec![(t.lang.clone(), String::new())]).unwrap_or_default()
        };
        let embed: Vec<(String, PathBuf)> = if o.subs == "embed" || o.subs == "embed_srt" { sub_files.clone() } else { vec![] };
        let cut = task.trim.map(|t| format!(" {}", trim_tag(t))).unwrap_or_default();

        let dest;
        match c.vcodec {
            None => {
                let orig = info.vcodec.clone().unwrap_or_else(|| "orig".into());
                if !embed.is_empty() || chosen.len() > 1 || !audio_meta.is_empty() || task.trim.is_some() {
                    // перепаковка без перекодирования: вшить субтитры, подписать дорожки, вырезать фрагмент
                    dest = unique_path(&out_dir.join(format!("{stem}{cut} [{q} {orig}{a_tag}].mkv")));
                    self.convert(task, &src, &dest, &info, "copy", "copy", &[], &embed, &audio_meta, false)?;
                } else {
                    let suffix = stem_suffix(&src).1;
                    dest = unique_path(&out_dir.join(format!("{stem} [{q} {orig}{a_tag}]{suffix}")));
                    move_file(&src, &dest)?;
                }
            }
            Some(vcodec) => {
                let mut ext = c.ext.unwrap_or("mp4");
                let mut extra: Vec<String> = c.extra.iter().map(|s| s.to_string()).collect();
                if !embed.is_empty() {
                    // со встроенными субтитрами — MKV: в MP4 первые субтитры всегда «включены по умолчанию»,
                    // а в MKV они выключены, пока их не выберут в плеере
                    ext = "mkv";
                    let mut cleaned = vec![];
                    for (i, a) in extra.iter().enumerate() {
                        if a != "-tag:v" && (i == 0 || extra[i - 1] != "-tag:v") {
                            cleaned.push(a.clone()); // hvc1 — только для mp4
                        }
                    }
                    extra = cleaned;
                }
                let tag = c.tag.unwrap_or("");
                dest = unique_path(&out_dir.join(format!("{stem}{cut} [{q} {tag}{a_tag}].{ext}")));
                self.convert(task, &src, &dest, &info, vcodec, c.acodec.unwrap_or("aac"), &extra, &embed, &audio_meta, video_copy)?;
            }
        }

        if !embed.is_empty() {
            self.log(self.t("py.embedded", &[("list", &embed.iter().map(|(l, _)| l.as_str()).collect::<Vec<_>>().join(", "))]), LogLevel::Normal);
        }
        task.srt_saved.clear();
        if o.subs == "embed_srt" {
            let dstem = stem_suffix(&dest).0;
            for (lang, f) in &sub_files {
                let target = unique_path(&dest.with_file_name(format!("{dstem}.{lang}.srt")));
                if let Some(trim) = task.trim {
                    // субтитры к фрагменту — тоже со сдвигом времени
                    let mut a = sv(&["ffmpeg", "-y", "-hide_banner", "-loglevel", "error"]);
                    a.extend(trim_in(Some(trim)));
                    a.extend(["-i".into(), f.to_string_lossy().into_owned()]);
                    a.extend(trim_out(Some(trim)));
                    a.push(target.to_string_lossy().into_owned());
                    self.run_proc(&a, |l| self.log(l, LogLevel::Error), "tc")?;
                } else {
                    move_file(f, &target)?;
                }
                task.srt_saved.push(target.to_string_lossy().into_owned());
                let name = target.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
                self.log(self.t("py.srtSaved", &[("name", &name)]), LogLevel::Normal);
            }
        }
        Ok(dest)
    }

    fn finalize_audio(&self, task: &mut Task, src: PathBuf, lang: Option<String>, title: String, artist: String) -> R<PathBuf> {
        let o = &self.opts;
        let out_dir = self.out_dir(task)?;
        let stem = self.stem(task, &src) + &task.trim.map(|t| format!(" {}", trim_tag(t))).unwrap_or_default();
        let a = probe(&src);
        let first = a.acodecs.first().cloned().unwrap_or_default();
        let target = audio_out(&o.codec).map(|_| o.codec.as_str());
        let mut copy = false;
        let (ext, args): (&str, Vec<String>) = match target {
            // блоки одного размера: иначе после обрезки AAC первый короткий кадр ломает кодировщик FLAC
            Some("flac") => ("flac", sv(&["-af", "asetnsamples=n=4096:p=0", "-c:a", "flac"])),
            Some("mp3") => {
                copy = first == "mp3";
                ("mp3", if copy { sv(&["-c:a", "copy"]) } else { sv(&["-c:a", "libmp3lame", "-b:a", &o.audio]) })
            }
            // M4A или «звук как есть» — AAC/MP3 с сайта не пережимаем
            _ if first == "aac" || first == "mp3" => {
                copy = true;
                (if first == "aac" { "m4a" } else { "mp3" }, sv(&["-c:a", "copy"]))
            }
            _ => ("m4a", sv(&["-c:a", "aac", "-b:a", &o.audio])),
        };
        if copy {
            self.emit(Event::TcMode { index: task.index, copy: true });
        }
        let dest = unique_path(&out_dir.join(format!("{stem}.{ext}")));
        let mut meta = vec!["-map_metadata".into(), "-1".into(), "-metadata".into(), format!("title={title}")];
        if !artist.is_empty() {
            meta.extend(["-metadata".into(), format!("artist={artist}")]);
        }
        if let Some(l) = lang.as_deref().filter(|l| !l.is_empty() && *l != "und") {
            meta.extend(["-metadata:s:a:0".into(), format!("language={}", iso3(l))]);
        }
        let mut cmd = sv(&["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1"]);
        cmd.extend(trim_in(task.trim));
        cmd.extend(["-i".into(), src.to_string_lossy().into_owned(), "-vn".into(), "-map".into(), "0:a:0".into()]);
        cmd.extend(trim_out(task.trim));
        cmd.extend(args);
        cmd.extend(meta);
        if ext == "m4a" {
            cmd.extend(sv(&["-movflags", "+faststart"]));
        }
        cmd.push(dest.to_string_lossy().into_owned());
        let dur = trim_duration(task.trim, a.duration);
        self.ffmpeg(task, &cmd, &dest, dur, None)?;
        task.srt_saved.clear();
        Ok(dest)
    }

    #[allow(clippy::too_many_arguments)]
    fn convert(&self, task: &Task, src: &Path, dest: &Path, info: &Probe, vcodec: &str, acodec: &str, extra: &[String],
               subs: &[(String, PathBuf)], audio_meta: &[(String, String)], video_copy: bool) -> R<()> {
        if video_copy {
            self.emit(Event::TcMode { index: task.index, copy: true });
            return self.convert_inner(task, src, dest, info, vcodec, acodec, extra, subs, audio_meta, None, true);
        }
        let codec = match vcodec { "libx264" => Some("h264"), "libx265" => Some("h265"), _ => None };
        let gpu = if codec.is_some() && self.opts.encoder == "gpu" { self.sh.gpu_for_job() } else { None };
        if let (Some(g), Some(c)) = (&gpu, codec) {
            self.log(self.t("py.gpuUse", &[("vendor", g.vendor), ("enc", g.enc(c))]), LogLevel::Normal);
            match self.convert_inner(task, src, dest, info, vcodec, acodec, extra, subs, audio_meta, Some(g), false) {
                Err(JobError::Ffmpeg(e)) => {
                    self.log(self.t("py.gpuFail", &[("e", &e)]), LogLevel::Error);
                    self.emit_progress(true, Progress {
                        index: task.index, stage: Some(Stage::Transcode), pct: Some(0.0), frame: Some(0), frames: info.frames,
                        ..Default::default()
                    });
                }
                r => return r,
            }
        }
        self.convert_inner(task, src, dest, info, vcodec, acodec, extra, subs, audio_meta, None, false)
    }

    #[allow(clippy::too_many_arguments)]
    fn convert_inner(&self, task: &Task, src: &Path, dest: &Path, info: &Probe, vcodec: &str, acodec: &str, extra: &[String],
                     subs: &[(String, PathBuf)], audio_meta: &[(String, String)], gpu: Option<&Gpu>, video_copy: bool) -> R<()> {
        let o = &self.opts;
        let codec = match vcodec { "libx264" => Some("h264"), "libx265" => Some("h265"), _ => None };
        let cut_in = trim_in(task.trim);
        let mut args = sv(&["ffmpeg", "-y", "-hide_banner", "-loglevel", "error", "-nostats", "-progress", "pipe:1"]);
        args.extend(cut_in.iter().cloned());
        args.extend(["-i".into(), src.to_string_lossy().into_owned()]);
        for (_, f) in subs {
            args.extend(cut_in.iter().cloned());
            args.extend(["-i".into(), f.to_string_lossy().into_owned()]);
        }
        args.extend(sv(&["-map", "0:v:0", "-map", "0:a?"]));
        for i in 0..subs.len() {
            args.extend(["-map".into(), format!("{}:0", i + 1)]);
        }
        args.extend(trim_out(task.trim));
        if task.trim.is_some() {
            args.extend(sv(&["-map_chapters", "-1"]));
        }
        if vcodec == "copy" {
            args.extend(sv(&["-c:v", "copy", "-c:a", "copy"]));
        } else if video_copy {
            // видео уже H.264 — только перепаковка; звук AAC тоже копируем, иначе пережимаем в AAC (это быстро)
            args.extend(sv(&["-c:v", "copy"]));
            if !info.acodecs.is_empty() && info.acodecs.iter().all(|c| c == "aac") {
                args.extend(sv(&["-c:a", "copy"]));
            } else {
                args.extend(sv(&["-c:a", "aac", "-b:a", &o.audio]));
            }
        } else {
            match codec {
                Some(c) => args.extend(encode::encoder_args(c, o.crf, &o.preset, gpu)),
                None => args.extend(sv(&["-c:v", vcodec])),
            }
            args.extend(extra.iter().cloned());
            args.extend(sv(&["-c:a", acodec]));
            if acodec == "aac" {
                args.extend(sv(&["-b:a", &o.audio]));
            }
        }
        // подписываем дорожки языком — плееры и монтажки показывают его в списке
        for (i, (lang, title)) in audio_meta.iter().enumerate() {
            args.extend([format!("-metadata:s:a:{i}"), format!("language={}", iso3(lang)),
                         format!("-disposition:a:{i}"), (if i == 0 { "default" } else { "0" }).into()]);
            if !title.is_empty() {
                args.extend([format!("-metadata:s:a:{i}"), format!("title={title}")]);
            }
        }
        let suffix = stem_suffix(dest).1;
        if !subs.is_empty() {
            args.extend(["-c:s".into(), (if suffix == ".mkv" { "srt" } else { "mov_text" }).into()]);
            for (i, (lang, _)) in subs.iter().enumerate() {
                args.extend([format!("-metadata:s:s:{i}"), format!("language={}", iso3(lang)), format!("-disposition:s:{i}"), "0".into()]);
            }
        }
        if suffix == ".mp4" {
            args.extend(sv(&["-movflags", "+faststart"]));
        }
        args.push(dest.to_string_lossy().into_owned());
        let dur = trim_duration(task.trim, info.duration);
        let frames = match dur { Some(d) if d != 0.0 && info.fps != 0.0 => Some((d * info.fps) as u64), _ => info.frames };
        self.ffmpeg(task, &args, dest, dur, frames)
    }

    fn ffmpeg(&self, task: &Task, args: &[String], dest: &Path, dur: Option<f64>, frames: Option<u64>) -> R<()> {
        let t0 = Instant::now();
        let (mut frame, mut speed): (u64, Option<f64>) = (0, None);
        let mut errors: Vec<String> = vec![];
        let idx = task.index;
        let dur = dur.filter(|d| *d != 0.0);
        let code = self.run_proc(args, |line| {
            let Some((k, v)) = line.split_once('=').filter(|(k, _)| !k.contains(' ')) else {
                errors.push(line.to_string());
                self.log(line, LogLevel::Error);
                return;
            };
            match k {
                "frame" => frame = num_str(v).unwrap_or(0.0) as u64,
                "speed" => speed = num_str(v.trim_end_matches('x')),
                "out_time_us" => {
                    if let Some(d) = dur {
                        let sec = num_str(v).unwrap_or(0.0) / 1e6;
                        let pct = (sec / d * 100.0).clamp(0.0, 100.0);
                        let el = t0.elapsed().as_secs_f64();
                        let eta = (pct > 0.5).then(|| el * (100.0 - pct) / pct);
                        self.emit_progress(false, Progress {
                            index: idx, stage: Some(Stage::Transcode), pct: Some(pct), frame: Some(frame), frames, speed, eta,
                            ..Default::default()
                        });
                    }
                }
                "progress" if v == "end" => {
                    let sp = speed.map(py_float).unwrap_or_else(|| "None".into());
                    self.log(format!("frame={frame} speed={sp}x"), LogLevel::Normal);
                }
                _ => {}
            }
        }, "tc");
        // отмена: недописанный файл не оставляем в папке пользователя (Python оставлял)
        if let Err(JobError::Cancelled) = code {
            let _ = std::fs::remove_file(dest);
        }
        let code = code?;
        let size = std::fs::metadata(dest).map(|m| m.len()).unwrap_or(0);
        if code != 0 || size == 0 {
            let _ = std::fs::remove_file(dest);
            let tail = errors.last().map(|e| format!(": {e}")).unwrap_or_else(|| ".".into());
            return Err(JobError::Ffmpeg(self.t("py.ffmpegFail", &[]) + &tail));
        }
        self.emit(Event::StageDone { index: idx, stage: Stage::Transcode, size, elapsed: t0.elapsed().as_secs_f64(), merged: false });
        Ok(())
    }
}

/// str(float) в Python: 1.0 -> "1.0", 1.25 -> "1.25".
fn py_float(v: f64) -> String {
    if v.fract() == 0.0 && v.abs() < 1e16 { format!("{v:.1}") } else { format!("{v}") }
}
