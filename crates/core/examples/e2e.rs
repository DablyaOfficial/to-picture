//! Проверка движка на настоящих ссылках (нужны сеть, yt-dlp и ffmpeg).
//! TOPICTURE_DATA=<временная папка> TOPICTURE_NO_UPDATE=1 cargo run -p topicture-core --example e2e -- <out_dir> [сценарии]
//! Сценарии: versions preview playlist h264 trim mp3 flac pipeline pause cancel_tc
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use topicture_core::*;

const ZOO: &str = "https://www.youtube.com/watch?v=jNQXAC9IVRw";
const PLAYLIST: &str = "https://www.youtube.com/playlist?list=PL590L5WQmH8coVKyRYxxQo8NG9O5NVLRW";
const BUNNY: &str = "https://www.youtube.com/watch?v=aqz-KE-bpKQ";

type Log = Arc<Mutex<Vec<(Instant, Event)>>>;

fn engine() -> (Arc<Engine>, Log) {
    let log: Log = Arc::default();
    let l = log.clone();
    let t0 = Instant::now();
    let sink: EventSink = Arc::new(move |ev: Event| {
        let line = match &ev {
            Event::Progress(p) => format!("progress {:?} {:?} pct={:.1?} done={:?} total={:?} speed={:.2?} eta={:.1?} frame={:?}/{:?} a={:?}/{:?} {}",
                p.stage, p.step, p.pct, p.done, p.total, p.speed, p.eta, p.frame, p.frames, p.audio_i, p.audio_n, p.note.clone().unwrap_or_default()),
            Event::Meta { index, data } => format!("meta #{index} {} thumb={} bytes", data.title, data.thumb.as_ref().map_or(0, |t| t.len())),
            e => serde_json::to_string(e).unwrap(),
        };
        println!("[{:6.2}] {line}", t0.elapsed().as_secs_f64());
        l.lock().unwrap().push((Instant::now(), ev));
    });
    (Engine::new(sink), log)
}

fn settings(out: &str) -> Settings {
    Settings { out_dir: out.into(), lang: "en".into(), ..Settings::default() }
}

fn wait(e: &Engine, max: Duration) {
    let t = Instant::now();
    while e.is_running() && t.elapsed() < max {
        std::thread::sleep(Duration::from_millis(100));
    }
}

fn ffprobe(path: &str) -> String {
    let ff = paths::tool("ffprobe").unwrap();
    let o = std::process::Command::new(ff)
        .args(["-v", "error", "-show_entries", "format=duration:stream=codec_type,codec_name,width,height:stream_tags=language", "-of", "compact=p=0", path])
        .output().unwrap();
    String::from_utf8_lossy(&o.stdout).trim().replace('\n', " | ").replace('\r', "")
}

fn report(log: &Log) {
    for (_, ev) in log.lock().unwrap().iter() {
        if let Event::ItemDone { path, size, srt, .. } = ev {
            println!("==> DONE {path} ({size} bytes) srt={srt:?}\n    ffprobe: {}", ffprobe(path));
            for s in srt {
                println!("    srt {s}: {}", ffprobe(s));
            }
        }
        if let Event::Finished { .. } = ev {
            println!("==> {}", serde_json::to_string(ev).unwrap());
        }
    }
    log.lock().unwrap().clear();
}

fn run_job(out: &str, urls: &[&str], s: Settings, ov: HashMap<String, Override>) {
    let (e, log) = engine();
    let items = urls.iter().map(|u| Item { url: u.to_string(), ..Default::default() }).collect();
    assert!(e.start(items, s, ov));
    assert!(!e.start(vec![], settings(out), HashMap::new()), "second start must be refused");
    wait(&e, Duration::from_secs(600));
    report(&log);
}

/// Все процессы-потомки (по снимку Toolhelp) корней, запущенных очередью.
fn our_tree(e: &Engine) -> Vec<u32> {
    let mut v = vec![];
    for p in e.job_pids() {
        for c in procs::process_tree(p) {
            if !v.contains(&c) { v.push(c); }
        }
    }
    v
}

fn alive_pids(pids: &[u32]) -> Vec<u32> {
    pids.iter().copied().filter(|p| {
        let o = std::process::Command::new("tasklist").args(["/FI", &format!("PID eq {p}"), "/NH", "/FO", "CSV"]).output().unwrap();
        String::from_utf8_lossy(&o.stdout).contains(&format!("\"{p}\""))
    }).collect()
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let out = args.first().expect("out_dir").clone();
    let only: Vec<&str> = args[1..].iter().map(String::as_str).collect();
    let want = |s: &str| only.is_empty() || only.contains(&s);
    println!("data_dir = {}, bin = {}", paths::data_dir().display(), paths::bin_dir().display());

    if want("versions") {
        let (e, _) = engine();
        let i = e.init();
        println!("init: lang={} system={} missing={:?} portable={}", i.lang, i.system_lang, i.missing, i.portable);
        let t = Instant::now();
        println!("versions: {} ({:.1}s)", e.versions(), t.elapsed().as_secs_f64());
        let t = Instant::now();
        println!("gpu: {:?} ({:.1}s)", e.gpu_info(), t.elapsed().as_secs_f64());
        println!("tr: {}", e.tr("py.ytdlpExit", &[("code", "3")]));
    }
    if want("preview") {
        let (e, _) = engine();
        let t = Instant::now();
        match e.preview(ZOO) {
            PreviewResult::Video(v) => println!("preview ({:.1}s): {} | {} | {:?}s | {:?} views | {:?} | {}p fps {:?} {:?} | heights {:?} | tracks {:?} | subs {:?} auto {} | site {} yt {} | thumb_url {:?} thumb {} bytes (magic {:02x?}) | vfmts {}",
                t.elapsed().as_secs_f64(), v.title, v.channel, v.duration, v.views, v.date, v.height.unwrap_or(0), v.fps, v.vcodec, v.heights,
                v.tracks, v.subs, v.auto_subs, v.site, v.youtube, v.thumb_url, v.thumb.as_ref().map_or(0, |t| t.len()),
                v.thumb.as_ref().map(|t| t[..4].to_vec()), v.vfmts.len()),
            r => println!("preview: unexpected {r:?}"),
        }
        match e.preview("https://example.com/nothing-here") {
            PreviewResult::Error { error, raw } => println!("preview bad url: {error} | raw: {raw}"),
            r => println!("preview bad url: unexpected {r:?}"),
        }
    }
    if want("playlist") {
        let (e, _) = engine();
        let t = Instant::now();
        match e.preview(PLAYLIST) {
            PreviewResult::Playlist(p) => println!("playlist ({:.1}s): {} | {} | {} entries | {:?}s | thumb_url {:?} thumb {} bytes | first {:?}",
                t.elapsed().as_secs_f64(), p.title, p.channel, p.entries.len(), p.duration, p.thumb_url, p.thumb.as_ref().map_or(0, |t| t.len()), p.entries.first()),
            r => println!("playlist: unexpected {r:?}"),
        }
    }
    if want("h264") {
        println!("\n######## h264 full (subs embed_srt, cache from preview)");
        let (e, log) = engine();
        let _ = e.preview(ZOO); // кэш превью -> job берёт --load-info-json без второго -J
        let s = Settings { subs: "embed_srt".into(), ..settings(&out) };
        assert!(e.start(vec![Item { url: ZOO.into(), ..Default::default() }], s, HashMap::new()));
        wait(&e, Duration::from_secs(600));
        report(&log);
    }
    if want("trim") {
        println!("\n######## h264 trim 2–5 s (gpu if available)");
        let mut ov = HashMap::new();
        ov.insert(ZOO.to_string(), Override { trim: Some((2.0, Some(5.0))), audio: None });
        run_job(&out, &[ZOO], Settings { subs: "embed_srt".into(), ..settings(&out) }, ov);
    }
    if want("mp3") {
        println!("\n######## mp3");
        run_job(&out, &[ZOO], Settings { codec: "mp3".into(), ..settings(&out) }, HashMap::new());
    }
    if want("flac") {
        println!("\n######## flac trim 1–4");
        let mut ov = HashMap::new();
        ov.insert(ZOO.to_string(), Override { trim: Some((1.0, Some(4.0))), audio: None });
        run_job(&out, &[ZOO], Settings { codec: "flac".into(), ..settings(&out) }, ov);
    }
    if want("pipeline") {
        println!("\n######## pipeline: 2 items in a playlist folder, h265 cpu");
        let (e, log) = engine();
        let items = vec![
            Item { url: ZOO.into(), folder: "My: list?".into(), prefix: "01 ".into() },
            Item { url: format!("{ZOO}&t=1"), folder: "My: list?".into(), prefix: "02 ".into() },
        ];
        let s = Settings { codec: "h265".into(), encoder: "cpu".into(), preset: "veryfast".into(), ..settings(&out) };
        assert!(e.start(items, s, HashMap::new()));
        wait(&e, Duration::from_secs(600));
        report(&log);
    }
    if want("cookies") {
        println!("\n######## cookies file missing: preview and job");
        let s = Settings { cookies: "file".into(), cookies_file: "C:/nope/cookies.txt".into(), ..settings(&out) };
        s.save();
        let (e, _) = engine();
        match e.preview(ZOO) {
            PreviewResult::Error { error, raw } => println!("==> preview: {error} | raw: {raw}"),
            r => println!("==> preview: unexpected {r:?}"),
        }
        run_job(&out, &[ZOO], s, HashMap::new());
        settings(&out).save();
    }
    if want("pause") {
        println!("\n######## pause / resume / cancel during download (rate limit 1M)");
        let (e, log) = engine();
        let s = Settings { max_height: "720".into(), rate_limit: "1M".into(), subs: "off".into(), ..settings(&out) };
        assert!(e.start(vec![Item { url: BUNNY.into(), ..Default::default() }], s, HashMap::new()));
        // ждём, пока пойдут байты
        let t = Instant::now();
        loop {
            let n = log.lock().unwrap().iter().filter(|(_, ev)| matches!(ev, Event::Progress(p) if p.done.is_some_and(|d| d > 0))).count();
            if n >= 3 || t.elapsed() > Duration::from_secs(120) { break; }
            std::thread::sleep(Duration::from_millis(100));
        }
        let tree = our_tree(&e);
        println!("our tree before pause: {tree:?}");
        e.pause(true);
        let paused_at = Instant::now();
        std::thread::sleep(Duration::from_secs(5));
        let during: usize = log.lock().unwrap().iter().filter(|(t, ev)| *t > paused_at + Duration::from_millis(300) && matches!(ev, Event::Progress(_))).count();
        println!("==> progress events while paused (5 s): {during}");
        let done_before = last_done(&log);
        e.pause(false);
        std::thread::sleep(Duration::from_secs(4));
        let done_after = last_done(&log);
        println!("==> bytes before resume {done_before:?}, 4 s after resume {done_after:?}");
        let tree = our_tree(&e);
        println!("our tree before cancel: {tree:?}");
        let tc = Instant::now();
        e.cancel();
        wait(&e, Duration::from_secs(30));
        println!("==> job ended {:.2}s after cancel; leftover of our processes: {:?}", tc.elapsed().as_secs_f64(), alive_pids(&tree));
        report(&log);
    }
    if want("cancel_tc") {
        println!("\n######## cancel during transcode (h265 cpu slow, trim 0–90)");
        let (e, log) = engine();
        let mut ov = HashMap::new();
        ov.insert(BUNNY.to_string(), Override { trim: Some((0.0, Some(90.0))), audio: None });
        let s = Settings { codec: "h265".into(), encoder: "cpu".into(), max_height: "720".into(), subs: "off".into(), ..settings(&out) };
        assert!(e.start(vec![Item { url: BUNNY.into(), ..Default::default() }], s, ov));
        let t = Instant::now();
        loop {
            let n = log.lock().unwrap().iter().filter(|(_, ev)| matches!(ev, Event::Progress(p) if p.stage == Some(Stage::Transcode))).count();
            if n >= 3 || t.elapsed() > Duration::from_secs(300) || !e.is_running() { break; }
            std::thread::sleep(Duration::from_millis(100));
        }
        // пауза во время перекодирования тоже
        e.pause(true);
        let paused_at = Instant::now();
        std::thread::sleep(Duration::from_secs(3));
        let during: usize = log.lock().unwrap().iter().filter(|(t, ev)| *t > paused_at + Duration::from_millis(300) && matches!(ev, Event::Progress(_))).count();
        println!("==> transcode progress events while paused (3 s): {during}");
        let tree = our_tree(&e);
        println!("our tree (paused) before cancel: {tree:?}");
        e.cancel();
        wait(&e, Duration::from_secs(30));
        println!("==> leftover of our processes: {:?}", alive_pids(&tree));
        report(&log);
    }
}

fn last_done(log: &Log) -> Option<u64> {
    log.lock().unwrap().iter().rev().find_map(|(_, ev)| match ev { Event::Progress(p) => p.done, _ => None })
}
