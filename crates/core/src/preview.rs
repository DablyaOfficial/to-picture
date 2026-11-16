//! Превью ссылки: yt-dlp -J, карточка видео или список плейлиста (preview_data / playlist_data в Python).
use crate::errors::{friendly_error, login_re};
use crate::media::{audio_tracks, formats, is_audio_only, max_first, no_codec, official_subs, video_formats};
use crate::net::net_args;
use crate::util::{fetch_thumb, jstr, num, truthy};
use crate::{
    AudioTrack, PlaylistEntry, PlaylistPreview, PreviewResult, Settings, Shared, VideoFormat, VideoPreview, i18n, procs,
};
use regex::Regex;
use serde_json::Value;
use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

/// Данные превью годятся для скачивания 10 минут (ссылки на потоки потом протухают).
pub const INFO_CACHE_TTL: Duration = Duration::from_secs(10 * 60);

/// url -> (время, JSON от yt-dlp -J, получено со входом из браузера).
pub struct CacheEntry {
    pub at: Instant,
    pub json: Vec<u8>,
    pub browser: bool,
}

/// Домен понятнее внутренних имён yt-dlp («TwitchClips»): vk.com, clips.twitch.tv.
pub fn site_of(info: &Value) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    let r = R.get_or_init(|| Regex::new(r"^(www|m)\.").unwrap());
    let d = r.replace(jstr(info, "webpage_url_domain").unwrap_or(""), "").into_owned();
    if d.is_empty() { jstr(info, "extractor_key").unwrap_or("").to_string() } else { d }
}

fn is_youtube(info: &Value) -> bool {
    jstr(info, "extractor").unwrap_or("").to_lowercase().contains("youtube")
}

/// Как скачивать обложку: адрес прокси и таймаут; None — не скачивать (только адрес).
#[derive(Clone)]
pub struct ThumbFetch {
    pub proxy: String,
    pub timeout: Duration,
}

/// Карточка превью по ответу yt-dlp -J (её же показываем для видео из плейлиста во время загрузки).
pub fn preview_data(info: &Value, url: &str, fetch: Option<&ThumbFetch>) -> VideoPreview {
    let vids: Vec<&Value> =
        formats(info).iter().filter(|f| !no_codec(f, "vcodec") && truthy(num(f.get("height"))).is_some()).collect();
    let key = |f: &&Value| {
        (num(f.get("height")).unwrap_or(0.0), truthy(num(f.get("fps"))).unwrap_or(0.0), truthy(num(f.get("tbr"))).unwrap_or(0.0))
    };
    let best = max_first(vids.iter().copied(), key);
    let mut heights: Vec<u32> = vids.iter().filter_map(|f| num(f.get("height"))).map(|h| h as u32).collect();
    heights.sort_unstable();
    heights.dedup();
    let is_yt = is_youtube(info);
    let referer = jstr(info, "webpage_url").unwrap_or(url).to_string();
    let mut thumb_url = jstr(info, "thumbnail").map(str::to_string);
    let mut thumb = None;
    if let (Some(id), true) = (jstr(info, "id"), is_yt) {
        let hq = format!("https://i.ytimg.com/vi/{id}/hqdefault.jpg");
        thumb_url = Some(hq.clone());
        for t in info.get("thumbnails").and_then(Value::as_array).into_iter().flatten() {
            if let Some(u) = jstr(t, "url").filter(|u| u.contains("maxresdefault.jpg")) {
                thumb_url = Some(u.to_string());
            }
        }
        if let Some(fx) = fetch {
            // maxresdefault есть не у всех видео — тогда hqdefault
            thumb = fetch_thumb(thumb_url.as_deref().unwrap(), &referer, &fx.proxy, fx.timeout)
                .or_else(|| (thumb_url.as_deref() != Some(&hq)).then(|| fetch_thumb(&hq, &referer, &fx.proxy, fx.timeout)).flatten());
        }
    } else if let (Some(u), Some(fx)) = (thumb_url.as_deref(), fetch) {
        // многие сайты не отдают обложку без «своего» Referer — загружаем её сами
        thumb = fetch_thumb(u, &referer, &fx.proxy, fx.timeout);
    }
    let fps = best.and_then(|b| num(b.get("fps")));
    VideoPreview {
        url: url.to_string(),
        title: jstr(info, "title").or(jstr(info, "id")).unwrap_or("").to_string(),
        channel: jstr(info, "uploader").or(jstr(info, "channel")).or(jstr(info, "creator")).unwrap_or("").to_string(),
        duration: num(info.get("duration")),
        views: num(info.get("view_count")).map(|v| v as u64),
        date: jstr(info, "upload_date").map(str::to_string),
        thumb_url,
        thumb: thumb.map(Arc::new),
        height: best.and_then(|b| num(b.get("height"))).map(|h| h as u32),
        fps,
        vcodec: best.map(|b| jstr(b, "vcodec").unwrap_or("").split('.').next().unwrap_or("").to_string()),
        heights,
        vfmts: video_formats(info)
            .into_iter()
            .map(|f| VideoFormat { h: f.h, fps: Some(f.fps as f64), vc: f.vc, hdr: f.hdr, https: f.https, tbr: Some(f.tbr) })
            .collect(),
        tracks: audio_tracks(info, false)
            .into_iter()
            .map(|t| AudioTrack { lang: t.lang, original: t.original, auto: t.auto })
            .collect(),
        subs: official_subs(info),
        auto_subs: info.get("automatic_captions").and_then(Value::as_object).is_some_and(|m| !m.is_empty()),
        language: jstr(info, "language").map(str::to_string),
        audio_only: is_audio_only(info),
        site: site_of(info),
        youtube: is_yt,
        browser_login: false,
    }
}

fn entries(v: &Value) -> Vec<&Value> {
    v.get("entries").and_then(Value::as_array).map(|a| a.iter().filter(|e| !e.is_null()).collect()).unwrap_or_default()
}

/// Видео плейлиста; канал YouTube приходит вкладками («Видео», «Трансляции», «Shorts») — берём первую.
pub fn playlist_entries(pl: &Value) -> Vec<PlaylistEntry> {
    let ents = entries(pl);
    if !ents.is_empty() && ents.iter().all(|e| jstr(e, "_type") == Some("playlist")) {
        return playlist_entries(ents[0]);
    }
    let mut out = vec![];
    for e in ents {
        let Some(mut u) = jstr(e, "url").or(jstr(e, "webpage_url")).map(str::to_string) else { continue };
        let http = |s: &str| s.starts_with("http://") || s.starts_with("https://");
        if !http(&u) && jstr(e, "ie_key") == Some("Youtube") {
            u = format!("https://www.youtube.com/watch?v={u}");
        }
        if !http(&u) {
            continue;
        }
        out.push(PlaylistEntry { url: u, title: jstr(e, "title").unwrap_or("").to_string(), duration: num(e.get("duration")) });
    }
    out
}

pub fn playlist_data(info: &Value, url: &str, lang: &str, fetch: Option<&ThumbFetch>) -> PreviewResult {
    let ents = playlist_entries(info);
    if ents.is_empty() {
        return PreviewResult::Error { error: i18n::tr(lang, "py.err.emptyPlaylist", &[]), raw: "empty playlist".into() };
    }
    let is_yt = is_youtube(info);
    let urls = |v: &Value| -> Vec<String> {
        v.get("thumbnails").and_then(Value::as_array).into_iter().flatten()
            .filter_map(|t| jstr(t, "url").map(str::to_string)).collect()
    };
    let mut thumbs = urls(info);
    if thumbs.is_empty() {
        if let Some(first) = info.get("entries").and_then(Value::as_array).and_then(|a| a.first()) {
            thumbs = urls(first);
        }
    }
    let mut thumb_url = thumbs.last().cloned();
    let referer = jstr(info, "webpage_url").unwrap_or(url).to_string();
    if is_yt && ents[0].url.contains("watch?v=") {
        let vid = ents[0].url.split("watch?v=").nth(1).unwrap_or("").split('&').next().unwrap_or("");
        thumb_url = Some(format!("https://i.ytimg.com/vi/{vid}/hqdefault.jpg"));
    }
    let thumb = match (thumb_url.as_deref(), fetch) {
        (Some(u), Some(fx)) => fetch_thumb(u, &referer, &fx.proxy, fx.timeout).map(Arc::new),
        _ => None,
    };
    let total: f64 = ents.iter().map(|e| e.duration.unwrap_or(0.0)).sum();
    PreviewResult::Playlist(PlaylistPreview {
        url: url.to_string(),
        title: jstr(info, "title").or(jstr(info, "id")).unwrap_or("").to_string(),
        channel: jstr(info, "uploader").or(jstr(info, "channel")).unwrap_or("").to_string(),
        site: site_of(info),
        youtube: is_yt,
        thumb_url,
        thumb,
        entries: ents,
        duration: truthy(Some(total)),
    })
}

fn last_error(stderr: &[u8]) -> String {
    let text = String::from_utf8_lossy(stderr);
    let lines: Vec<&str> = text.trim().lines().collect();
    lines.iter().rev().find(|l| l.starts_with("ERROR")).or(lines.last()).map(|s| s.to_string()).unwrap_or_default()
}

/// Engine::preview. Блокирующий вызов.
pub(crate) fn preview(sh: &Shared, url: &str) -> PreviewResult {
    let s = Settings::load();
    let lang = sh.lang();
    let err = |e: String, raw: String| PreviewResult::Error { error: e, raw };
    let tmp = match tempfile::Builder::new().prefix("topicture_pv_").tempdir() {
        Ok(t) => t,
        Err(e) => return err(friendly_error(&e.to_string(), &lang), e.to_string()),
    };
    let Ok(net) = net_args(&s, Some(tmp.path()), None) else {
        return err(i18n::tr(&lang, "py.err.cookieNoFile", &[]), "cookies file missing".into());
    };
    // --flat-playlist: у плейлиста и канала — только список видео, без данных каждого (быстро)
    let cmd = |net: Vec<String>| -> Vec<String> {
        let mut a: Vec<String> =
            ["yt-dlp", "-J", "--flat-playlist", "--no-playlist", "--no-warnings"].iter().map(|s| s.to_string()).collect();
        a.extend(net);
        a.push("--".into());
        a.push(url.into());
        a
    };
    let run = |a: Vec<String>| procs::run_quiet(&a, Duration::from_secs(180));
    let mut r = match run(cmd(net)) {
        Ok(r) => r,
        Err(e) => return err(friendly_error(&e.to_string(), &lang), e.to_string()),
    };
    let mut browser = false;
    if r.code != Some(0) && s.cookies == "off" && login_re().is_match(&String::from_utf8_lossy(&r.stderr)) {
        if let Some(bc) = sh.cookies.for_url(url) {
            // нужен вход — пробуем cookies, которые прислало расширение вместе со ссылкой
            let net = net_args(&s, Some(tmp.path()), Some(&bc)).unwrap_or_default();
            r = match run(cmd(net)) {
                Ok(r) => r,
                Err(e) => return err(friendly_error(&e.to_string(), &lang), e.to_string()),
            };
            browser = r.code == Some(0);
        }
    }
    drop(tmp);
    if r.code != Some(0) {
        let e = last_error(&r.stderr);
        return err(friendly_error(&e, &lang), e);
    }
    let info: Value = match serde_json::from_str(&String::from_utf8_lossy(&r.stdout)) {
        Ok(v) => v,
        Err(e) => return err(i18n::tr(&lang, "py.err.unknown", &[]), e.to_string()),
    };
    let fx = ThumbFetch { proxy: s.proxy.clone(), timeout: Duration::from_secs(10) };
    if jstr(&info, "_type") == Some("playlist") {
        return playlist_data(&info, url, &lang, Some(&fx));
    }
    sh.info_cache.lock().unwrap().insert(url.to_string(), CacheEntry { at: Instant::now(), json: r.stdout, browser });
    // публикация без видео и звука (например, пост с картинками) — говорим сразу, а не при скачивании
    let no_formats = info.get("formats").and_then(Value::as_array).is_none_or(|a| a.is_empty());
    if no_formats && jstr(&info, "url").is_none() {
        return err(i18n::tr(&lang, "py.err.noVideo", &[]), "no formats".into());
    }
    let mut d = preview_data(&info, url, Some(&fx));
    d.browser_login = browser;
    PreviewResult::Video(d)
}
