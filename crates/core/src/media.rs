//! Выбор видеопотока, озвучек и субтитров по ответу yt-dlp -J (как в ToPicture.pyw).
use crate::util::{base_lang, num, truthy};
use serde_json::Value;

/// Видеопоток без звука в компактном виде.
#[derive(Debug, Clone, PartialEq)]
pub struct VFmt {
    pub id: String,
    pub h: u32,
    pub fps: i64,
    pub vc: String,
    pub hdr: bool,
    pub https: bool,
    pub tbr: f64,
}

/// Озвучка: лучшая дорожка на язык.
#[derive(Debug, Clone, PartialEq)]
pub struct Track {
    pub lang: String,
    pub id: String,
    pub original: bool,
    pub auto: bool,
}

/// Правило озвучек: из настроек (orig | orig_ui | orig_ui_en | all) или список языков для одной ссылки.
#[derive(Debug, Clone, PartialEq)]
pub enum AudioRule {
    Rule(String),
    Langs(Vec<String>),
}

impl AudioRule {
    /// Override.audio: имя правила или языки через запятую («en,ru»).
    pub fn parse(s: &str) -> AudioRule {
        if crate::settings::AUDIO_RULES.contains(&s) {
            AudioRule::Rule(s.into())
        } else {
            AudioRule::Langs(s.split(',').map(|x| x.trim().to_string()).filter(|x| !x.is_empty()).collect())
        }
    }
}

/// Поле кодека отсутствует / null / "none".
pub(crate) fn no_codec(f: &Value, key: &str) -> bool {
    match f.get(key) {
        None | Some(Value::Null) => true,
        Some(Value::String(s)) => s == "none",
        _ => false,
    }
}

fn s<'a>(f: &'a Value, key: &str) -> &'a str {
    f.get(key).and_then(Value::as_str).unwrap_or("")
}

pub(crate) fn formats(info: &Value) -> &[Value] {
    info.get("formats").and_then(Value::as_array).map(Vec::as_slice).unwrap_or(&[])
}

/// Первый максимальный элемент (как max() в Python при равных ключах).
pub(crate) fn max_first<T, K: PartialOrd>(it: impl IntoIterator<Item = T>, key: impl Fn(&T) -> K) -> Option<T> {
    it.into_iter().reduce(|a, b| if key(&b) > key(&a) { b } else { a })
}

/// Ни одного видеопотока — например, SoundCloud или подкаст.
pub fn is_audio_only(info: &Value) -> bool {
    let f = formats(info);
    if !f.is_empty() {
        return f.iter().all(|x| x.get("vcodec").and_then(Value::as_str) == Some("none"));
    }
    info.get("vcodec").and_then(Value::as_str) == Some("none")
}

pub fn video_formats(info: &Value) -> Vec<VFmt> {
    let mut out = vec![];
    for f in formats(info) {
        let h = truthy(num(f.get("height")));
        if no_codec(f, "vcodec") || h.is_none() || !no_codec(f, "acodec") {
            continue;
        }
        let proto = s(f, "protocol");
        out.push(VFmt {
            id: s(f, "format_id").to_string(),
            h: h.unwrap_or(0.0) as u32,
            fps: num(f.get("fps")).unwrap_or(0.0).round_ties_even() as i64,
            vc: s(f, "vcodec").split('.').next().unwrap_or("").to_string(),
            hdr: f.get("dynamic_range").and_then(Value::as_str).filter(|x| !x.is_empty()).unwrap_or("SDR") != "SDR",
            https: proto.starts_with("http") && !proto.contains("m3u8"),
            tbr: num(f.get("tbr")).unwrap_or(0.0),
        });
    }
    out
}

/// Какой видеопоток качать. (формат или None, copy_video).
pub fn select_video(vfmts: &[VFmt], max_height: &str, codec: &str, allow_copy: bool) -> (Option<VFmt>, bool) {
    if vfmts.is_empty() {
        return (None, false);
    }
    let lim: Option<u32> = if max_height.is_empty() || max_height == "best" { None } else { max_height.trim().parse().ok() };
    let lim = lim.filter(|l| *l != 0);
    let mut pool: Vec<&VFmt> = vfmts.iter().filter(|f| lim.is_none_or(|l| f.h <= l)).collect();
    if pool.is_empty() {
        pool = vfmts.iter().collect();
    }
    let key = |f: &&VFmt| (f.h, f.fps, f.https, f.tbr);
    if codec == "copy" {
        return (max_first(pool, key).cloned(), false);
    }
    let mut sdr: Vec<&VFmt> = pool.iter().copied().filter(|f| !f.hdr).collect();
    if sdr.is_empty() {
        sdr = pool;
    }
    let best = max_first(sdr.iter().copied(), key).unwrap();
    if codec == "h264" && allow_copy {
        let avc: Vec<&VFmt> =
            sdr.iter().copied().filter(|f| f.vc.starts_with("avc1") && f.h == best.h && f.fps == best.fps).collect();
        if let Some(a) = max_first(avc, key) {
            return (Some(a.clone()), true);
        }
    }
    (Some(best.clone()), false)
}

/// Озвучки видео: по одной лучшей дорожке на язык (без DRC — сжатой динамики).
pub fn audio_tracks(info: &Value, prefer_aac: bool) -> Vec<Track> {
    // порядок языков — порядок первого появления (как dict в Python)
    let mut by_lang: Vec<(String, (bool, bool, f64), &Value)> = vec![];
    for f in formats(info) {
        if !no_codec(f, "vcodec") || no_codec(f, "acodec") {
            continue;
        }
        let lang = f.get("language").and_then(Value::as_str).filter(|x| !x.is_empty()).unwrap_or("und").to_string();
        let note = s(f, "format_note").to_lowercase();
        let key = (
            !note.contains("drc"),
            prefer_aac && s(f, "acodec").starts_with("mp4a"),
            truthy(num(f.get("abr"))).or(truthy(num(f.get("tbr")))).unwrap_or(0.0),
        );
        match by_lang.iter_mut().find(|(l, _, _)| *l == lang) {
            None => by_lang.push((lang, key, f)),
            Some(cur) => {
                if key > cur.1 {
                    cur.1 = key;
                    cur.2 = f;
                }
            }
        }
    }
    let mut tracks: Vec<Track> = by_lang
        .into_iter()
        .map(|(lang, _, f)| {
            let note = s(f, "format_note").to_lowercase();
            Track {
                lang,
                id: s(f, "format_id").to_string(),
                original: num(f.get("language_preference")).unwrap_or(0.0) >= 10.0 || note.contains("original"),
                auto: note.contains("auto"),
            }
        })
        .collect();
    tracks.sort_by(|a, b| (!a.original, &a.lang).cmp(&(!b.original, &b.lang)));
    tracks
}

/// Дорожки для встраивания. Оригинал всегда первый.
pub fn pick_audio(tracks: &[Track], rule: &AudioRule, ui: &str) -> Vec<Track> {
    if tracks.len() <= 1 {
        return tracks.to_vec();
    }
    if matches!(rule, AudioRule::Rule(r) if r == "all") {
        return tracks.to_vec();
    }
    let orig = tracks.iter().find(|t| t.original).unwrap_or(&tracks[0]).clone();
    match rule {
        AudioRule::Langs(list) => {
            let want: Vec<String> = list.iter().map(|x| base_lang(x)).collect();
            let pos = |t: &Track| want.iter().position(|w| *w == base_lang(&t.lang)).unwrap_or(usize::MAX);
            let mut out: Vec<Track> = tracks.iter().filter(|t| want.contains(&base_lang(&t.lang))).cloned().collect();
            out.sort_by_key(|t| (!t.original, pos(t)));
            if out.is_empty() { vec![orig] } else { out }
        }
        AudioRule::Rule(r) => {
            let ui = base_lang(ui);
            let extra: Vec<String> = match r.as_str() {
                "orig" => vec![],
                "orig_ui" => vec![ui],
                "orig_ui_en" => vec![ui, "en".into()],
                _ => vec![ui],
            };
            let mut out = vec![orig];
            for lang in extra {
                if let Some(t) = tracks.iter().find(|t| base_lang(&t.lang) == lang) {
                    if !out.contains(t) {
                        out.push(t.clone());
                    }
                }
            }
            out
        }
    }
}

pub fn official_subs(info: &Value) -> Vec<String> {
    let mut v: Vec<String> = info
        .get("subtitles")
        .and_then(Value::as_object)
        .map(|m| m.keys().filter(|k| *k != "live_chat").cloned().collect())
        .unwrap_or_default();
    v.sort();
    v
}

/// Коды официальных субтитров для встраивания — по одному на язык.
pub fn pick_subs(available: &[String], rule: &str, audio_langs: &[String], orig_lang: Option<&str>, ui: &str) -> Vec<String> {
    let one = |lang: &str| -> Option<String> {
        let mut m: Vec<&String> = available.iter().filter(|k| base_lang(k) == lang).collect();
        m.sort_by(|a, b| (a.as_str() != lang, a.as_str()).cmp(&(b.as_str() != lang, b.as_str())));
        m.first().map(|s| s.to_string())
    };
    if rule == "all" {
        return available.to_vec();
    }
    let langs: Vec<String> = if rule == "audio" {
        let l: Vec<String> = audio_langs.iter().map(|x| base_lang(x)).collect();
        if l.is_empty() { vec![base_lang(orig_lang.unwrap_or(""))] } else { l }
    } else {
        let ui = base_lang(ui);
        match rule {
            "ui" => vec![ui],
            "en" => vec!["en".into()],
            _ => vec![ui, "en".into()],
        }
    };
    let mut seen: Vec<String> = vec![];
    let mut out = vec![];
    for l in langs.into_iter().filter(|l| !l.is_empty()) {
        if seen.contains(&l) {
            continue;
        }
        seen.push(l.clone());
        out.extend(one(&l));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn info() -> Value {
        json!({"formats": [
            {"format_id": "sb0", "vcodec": "none", "acodec": "none", "height": 90},
            {"format_id": "140", "vcodec": "none", "acodec": "mp4a.40.2", "abr": 129.5, "language": "en", "format_note": "English original, medium", "language_preference": 10},
            {"format_id": "251", "vcodec": "none", "acodec": "opus", "abr": 135.0, "language": "en", "format_note": "English original, medium"},
            {"format_id": "251-drc", "vcodec": "none", "acodec": "opus", "abr": 140.0, "language": "en", "format_note": "English original, medium, DRC"},
            {"format_id": "251-1", "vcodec": "none", "acodec": "opus", "abr": 130.0, "language": "ru", "format_note": "Russian (auto-dubbed), medium"},
            {"format_id": "251-2", "vcodec": "none", "acodec": "opus", "abr": 130.0, "language": "de-DE", "format_note": "German (auto-dubbed)"},
            {"format_id": "137", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "fps": 30, "tbr": 4000.0, "protocol": "https"},
            {"format_id": "248", "vcodec": "vp9", "acodec": "none", "height": 1080, "fps": 30, "tbr": 2500.0, "protocol": "https"},
            {"format_id": "399", "vcodec": "av01.0.08M.08", "acodec": "none", "height": 1080, "fps": 30, "tbr": 2000.0, "protocol": "https"},
            {"format_id": "337", "vcodec": "vp09.02.51.10", "acodec": "none", "height": 2160, "fps": 60, "tbr": 20000.0, "dynamic_range": "HDR10", "protocol": "https"},
            {"format_id": "315", "vcodec": "vp9", "acodec": "none", "height": 2160, "fps": 60, "tbr": 15000.0, "protocol": "https"},
            {"format_id": "hls-1080", "vcodec": "avc1.640028", "acodec": "none", "height": 1080, "fps": 30, "tbr": 9000.0, "protocol": "m3u8_native"},
            {"format_id": "18", "vcodec": "avc1.42001E", "acodec": "mp4a.40.2", "height": 360, "fps": 30, "protocol": "https"}
        ], "subtitles": {"en": [], "en-GB": [], "ru": [], "live_chat": [], "de": []}})
    }

    #[test]
    fn video_pick() {
        let v = video_formats(&info());
        assert_eq!(v.len(), 6);
        // лучший SDR — 2160p60 vp9; avc1 того же размера нет -> перекодирование
        let (f, copy) = select_video(&v, "best", "h264", true);
        assert_eq!((f.unwrap().id.as_str(), copy), ("315", false));
        // «без перекодирования» берёт HDR
        assert_eq!(select_video(&v, "best", "copy", true).0.unwrap().id, "337");
        // 1080: есть avc1 той же высоты и fps -> копия; https важнее m3u8, хотя у hls tbr выше
        let (f, copy) = select_video(&v, "1080", "h264", true);
        assert_eq!((f.unwrap().id.as_str(), copy), ("137", true));
        // с фрагментом копирование запрещено -> лучший по (h, fps, https, tbr) — тоже 137 (tbr выше vp9)
        let (f, copy) = select_video(&v, "1080", "h264", false);
        assert_eq!((f.unwrap().id.as_str(), copy), ("137", false));
        let (f, copy) = select_video(&v, "1080", "h265", true);
        assert_eq!((f.unwrap().id.as_str(), copy), ("137", false));
        // лимит ниже всех — берётся весь список
        assert_eq!(select_video(&v, "144", "h265", true).0.unwrap().id, "315");
        assert_eq!(select_video(&[], "best", "h264", true), (None, false));
    }

    #[test]
    fn audio_pick() {
        let t = audio_tracks(&info(), false);
        let ids: Vec<_> = t.iter().map(|x| (x.lang.as_str(), x.id.as_str(), x.original, x.auto)).collect();
        assert_eq!(ids, vec![("en", "251", true, false), ("de-DE", "251-2", false, true), ("ru", "251-1", false, true)]);
        let t2 = audio_tracks(&info(), true);
        assert_eq!(t2[0].id, "140");
        let r = |x: &str| AudioRule::Rule(x.into());
        let langs = |v: &[Track]| v.iter().map(|x| x.lang.clone()).collect::<Vec<_>>();
        assert_eq!(langs(&pick_audio(&t, &r("orig"), "ru")), ["en"]);
        assert_eq!(langs(&pick_audio(&t, &r("orig_ui"), "ru")), ["en", "ru"]);
        assert_eq!(langs(&pick_audio(&t, &r("orig_ui"), "en")), ["en"]);
        assert_eq!(langs(&pick_audio(&t, &r("orig_ui_en"), "de")), ["en", "de-DE"]);
        assert_eq!(langs(&pick_audio(&t, &r("all"), "de")), ["en", "de-DE", "ru"]);
        assert_eq!(langs(&pick_audio(&t, &AudioRule::parse("ru,de"), "en")), ["ru", "de-DE"]);
        assert_eq!(langs(&pick_audio(&t, &AudioRule::parse("ru,en"), "en")), ["en", "ru"]);
        assert_eq!(langs(&pick_audio(&t, &AudioRule::parse("fr"), "en")), ["en"]);
        assert_eq!(AudioRule::parse("all"), r("all"));
        assert!(!is_audio_only(&info()));
        assert!(is_audio_only(&json!({"formats": [{"vcodec": "none"}]})));
        assert!(is_audio_only(&json!({"vcodec": "none"})));
    }

    #[test]
    fn subs_pick() {
        let a = official_subs(&info());
        assert_eq!(a, ["de", "en", "en-GB", "ru"]);
        assert_eq!(pick_subs(&a, "ui_en", &[], None, "ru-RU"), ["ru", "en"]);
        assert_eq!(pick_subs(&a, "ui_en", &[], None, "en"), ["en"]);
        assert_eq!(pick_subs(&a, "ui", &[], None, "fr"), Vec::<String>::new());
        assert_eq!(pick_subs(&a, "en", &[], None, "ru"), ["en"]);
        assert_eq!(pick_subs(&a, "all", &[], None, "ru"), a);
        assert_eq!(pick_subs(&a, "audio", &["de-DE".into(), "en".into()], None, "ru"), ["de", "en"]);
        assert_eq!(pick_subs(&a, "audio", &[], Some("ru"), "en"), ["ru"]);
        let gb = vec!["en-GB".to_string(), "en-US".to_string()];
        assert_eq!(pick_subs(&gb, "en", &[], None, "en"), ["en-GB"]);
    }
}
