//! Мелкие помощники: имена файлов, фрагменты (trim), разбор чисел из JSON, обложки.
use regex::Regex;
use serde_json::Value;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

/// Фрагмент: (начало, конец или None) в секундах.
pub type Trim = (f64, Option<f64>);

pub(crate) fn re(cell: &'static OnceLock<Regex>, pat: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pat).expect("regex"))
}

/// Имя папки из названия плейлиста: без символов, запрещённых в Windows.
pub fn safe_name(s: &str, limit: usize) -> String {
    static R: OnceLock<Regex> = OnceLock::new();
    let s = re(&R, r#"[<>:"/\\|?*\x00-\x1f]"#).replace_all(s, "_");
    let s = s.trim().trim_end_matches(['.', ' ']);
    let cut: String = s.chars().take(limit).collect();
    cut.trim_end_matches(['.', ' ']).to_string()
}

/// Число из JSON: число или строка с числом (как float(v) в Python). "NA", null -> None.
pub fn num(v: Option<&Value>) -> Option<f64> {
    match v? {
        Value::Number(n) => n.as_f64(),
        Value::String(s) => num_str(s),
        Value::Bool(b) => Some(if *b { 1.0 } else { 0.0 }),
        _ => None,
    }
}

/// float(s) из Python: "12", "1.5e3", " 7 ", "inf"; "NA"/"N/A" -> None.
pub fn num_str(s: &str) -> Option<f64> {
    let t = s.trim();
    let v: f64 = t.parse().ok()?;
    if v.is_nan() && !t.eq_ignore_ascii_case("nan") { None } else { Some(v) }
}

/// «Истинное» число Python: не None и не 0.
pub fn truthy(v: Option<f64>) -> Option<f64> {
    v.filter(|x| *x != 0.0)
}

pub fn trim_in(trim: Option<Trim>) -> Vec<String> {
    match trim {
        Some((a, _)) if a != 0.0 => vec!["-ss".into(), format!("{a:.3}")],
        _ => vec![],
    }
}

pub fn trim_out(trim: Option<Trim>) -> Vec<String> {
    match trim {
        Some((a, Some(b))) if b != 0.0 => vec!["-t".into(), format!("{:.3}", b - a)],
        _ => vec![],
    }
}

pub fn trim_duration(trim: Option<Trim>, dur: Option<f64>) -> Option<f64> {
    let Some((a, b)) = trim else { return dur };
    let end = truthy(b).or(truthy(dur))?;
    Some((end - a).max(0.0))
}

/// (1.20–3.45) — в имени файла фрагмента (двоеточие в Windows запрещено).
pub fn trim_tag(trim: Trim) -> String {
    fn f(s: f64) -> String {
        let s = s.round_ties_even() as i64;
        let (h, m, ss) = (s / 3600, s % 3600 / 60, s % 60);
        if h != 0 { format!("{h}.{m:02}.{ss:02}") } else { format!("{m}.{ss:02}") }
    }
    let (a, b) = trim;
    let end = match truthy(b) { Some(b) => f(b), None => "end".into() };
    format!("({}–{})", f(a), end)
}

pub fn quality_tag(height: Option<u32>, fps: f64) -> String {
    match height {
        Some(h) if h != 0 => {
            let f = if fps > 31.0 { format!("{}", fps.round_ties_even() as i64) } else { String::new() };
            format!("{h}p{f}")
        }
        _ => String::new(),
    }
}

/// Имя занято — добавляем _ГГГГММДД_ЧЧММСС.
pub fn unique_path(p: &Path) -> PathBuf {
    if !p.exists() {
        return p.to_path_buf();
    }
    let (stem, suffix) = stem_suffix(p);
    p.with_file_name(format!("{stem}_{}{suffix}", local_stamp()))
}

/// (stem, suffix) как Path.stem / Path.suffix в Python (suffix с точкой или пустой).
pub fn stem_suffix(p: &Path) -> (String, String) {
    let name = p.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() => (name[..i].to_string(), name[i..].to_string()),
        _ => (name, String::new()),
    }
}

fn local_stamp() -> String {
    use windows_sys::Win32::System::SystemInformation::GetLocalTime;
    let t = unsafe {
        let mut t = std::mem::zeroed();
        GetLocalTime(&mut t);
        t
    };
    format!("{:04}{:02}{:02}_{:02}{:02}{:02}", t.wYear, t.wMonth, t.wDay, t.wHour, t.wMinute, t.wSecond)
}

/// Переместить файл (между дисками — копированием), как shutil.move.
pub fn move_file(src: &Path, dst: &Path) -> std::io::Result<()> {
    if std::fs::rename(src, dst).is_ok() {
        return Ok(());
    }
    std::fs::copy(src, dst)?;
    let _ = std::fs::remove_file(src);
    Ok(())
}

pub fn base_lang(code: &str) -> String {
    code.split('-').next().unwrap_or("").to_lowercase()
}

/// ISO 639-1 -> 639-2 для метаданных дорожек в mp4/mov (плееры показывают язык по ним).
pub fn iso3(code: &str) -> String {
    const ISO3: &[(&str, &str)] = &[
        ("ru", "rus"), ("en", "eng"), ("de", "ger"), ("fr", "fre"), ("es", "spa"), ("it", "ita"), ("pt", "por"),
        ("ja", "jpn"), ("ko", "kor"), ("zh", "chi"), ("ar", "ara"), ("hi", "hin"), ("pl", "pol"), ("tr", "tur"),
        ("uk", "ukr"), ("vi", "vie"), ("id", "ind"), ("th", "tha"), ("fa", "per"), ("nl", "dut"), ("sv", "swe"),
        ("cs", "cze"), ("el", "gre"), ("he", "heb"), ("hu", "hun"), ("ro", "rum"), ("fi", "fin"), ("da", "dan"),
        ("no", "nor"), ("nb", "nob"), ("bn", "ben"), ("ta", "tam"), ("te", "tel"), ("mr", "mar"), ("ml", "mal"),
        ("pa", "pan"), ("uz", "uzb"), ("kk", "kaz"), ("be", "bel"), ("ms", "may"), ("fil", "fil"), ("ur", "urd"),
    ];
    let b = base_lang(code);
    if b.is_empty() || b == "und" {
        return "und".into();
    }
    ISO3.iter().find(|(k, _)| *k == b).map(|(_, v)| v.to_string()).unwrap_or(b)
}

/// Строка из JSON-поля (пустая строка = нет, как `x or ...` в Python).
pub fn jstr<'a>(v: &'a Value, key: &str) -> Option<&'a str> {
    v.get(key).and_then(Value::as_str).filter(|s| !s.is_empty())
}

pub const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) \
                      Chrome/124.0 Safari/537.36";

/// Обложка (до 3 МБ), с Referer страницы видео. None — если не вышло.
/// proxy — из настроек (http/https/socks не поддержанный ureq — тогда системный).
pub fn fetch_thumb(url: &str, referer: &str, proxy: &str, timeout: Duration) -> Option<Vec<u8>> {
    const LIMIT: u64 = 3 * 1024 * 1024;
    let mut cfg = ureq::Agent::config_builder().timeout_global(Some(timeout)).http_status_as_error(true);
    let proxy = proxy.trim();
    if !proxy.is_empty() {
        if let Ok(p) = ureq::Proxy::new(proxy) {
            cfg = cfg.proxy(Some(p));
        }
    }
    let agent: ureq::Agent = cfg.build().into();
    let mut req = agent.get(url).header("User-Agent", UA);
    if !referer.is_empty() {
        req = req.header("Referer", referer);
    }
    let resp = req.call().ok()?;
    let data = resp.into_body().with_config().limit(LIMIT).read_to_vec().ok()?;
    (!data.is_empty()).then_some(data)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names() {
        assert_eq!(safe_name("a<b>:c\"/d\\|?*. . ", 120), "a_b__c__d____");
        assert_eq!(safe_name("  My list...  ", 120), "My list");
        assert_eq!(safe_name("abc. def", 5), "abc");
        assert_eq!(safe_name("Плейлист: часть 1", 120), "Плейлист_ часть 1");
    }

    #[test]
    fn trims() {
        assert_eq!(trim_in(Some((2.0, Some(5.0)))), vec!["-ss", "2.000"]);
        assert!(trim_in(Some((0.0, Some(5.0)))).is_empty());
        assert_eq!(trim_out(Some((2.0, Some(5.0)))), vec!["-t", "3.000"]);
        assert!(trim_out(Some((2.0, None))).is_empty());
        assert!(trim_out(None).is_empty());
        assert_eq!(trim_duration(Some((2.0, Some(5.0))), Some(19.0)), Some(3.0));
        assert_eq!(trim_duration(Some((2.0, None)), Some(19.0)), Some(17.0));
        assert_eq!(trim_duration(Some((2.0, None)), None), None);
        assert_eq!(trim_duration(None, Some(19.0)), Some(19.0));
        assert_eq!(trim_duration(Some((30.0, None)), Some(19.0)), Some(0.0));
        assert_eq!(trim_tag((2.0, Some(5.0))), "(0.02–0.05)");
        assert_eq!(trim_tag((80.0, None)), "(1.20–end)");
        assert_eq!(trim_tag((3725.0, Some(7322.4))), "(1.02.05–2.02.02)");
        assert_eq!(trim_tag((2.5, Some(3.5))), "(0.02–0.04)"); // банковское округление, как round() в Python
    }

    #[test]
    fn tags_and_langs() {
        assert_eq!(quality_tag(Some(1080), 30.0), "1080p");
        assert_eq!(quality_tag(Some(1080), 59.94), "1080p60");
        assert_eq!(quality_tag(None, 60.0), "");
        assert_eq!(iso3("en-US"), "eng");
        assert_eq!(iso3("und"), "und");
        assert_eq!(iso3(""), "und");
        assert_eq!(iso3("xx"), "xx");
        assert_eq!(num_str("NA"), None);
        assert_eq!(num_str("12.5"), Some(12.5));
        let (s, x) = stem_suffix(Path::new("C:/a/My.video.en.srt"));
        assert_eq!((s.as_str(), x.as_str()), ("My.video.en", ".srt"));
    }

    #[test]
    fn unique() {
        let d = tempfile::tempdir().unwrap();
        let p = d.path().join("x.mp4");
        assert_eq!(unique_path(&p), p);
        std::fs::write(&p, b"1").unwrap();
        let u = unique_path(&p);
        let n = u.file_name().unwrap().to_string_lossy().into_owned();
        assert!(n.starts_with("x_") && n.ends_with(".mp4") && n.len() == "x_20250101_120000.mp4".len(), "{n}");
    }
}
