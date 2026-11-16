//! Форматирование размеров, времени, просмотров и дат (fmtSize, fmtTime, fmtViews … из index.html),
//! разбор времени фрагмента (parseTime) и обрезка середины пути под ширину поля (fitMiddle).
use crate::icu;
use std::sync::OnceLock;
use topicture_core::i18n;

/// Перевод с параметрами.
pub fn t(lang: &str, key: &str, p: &[(&str, &str)]) -> String {
    i18n::tr(lang, key, p)
}

/// Число с d знаками после запятой, с разделителями языка (Intl.NumberFormat).
pub fn nf(lang: &str, d: usize, v: f64) -> String {
    let sk = match d { 0 => "precision-integer", 1 => ".0", _ => ".00" };
    icu::number(lang, sk, v).unwrap_or_else(|| {
        let s = format!("{v:.d$}");
        if matches!(base(lang).as_str(), "en" | "ja" | "ko" | "zh") { s } else { s.replace('.', ",") }
    })
}

pub fn base(code: &str) -> String {
    code.split('-').next().unwrap_or("").to_ascii_lowercase()
}

fn size_unit(lang: &str, b: f64) -> (f64, String, usize) {
    let g = 1024f64.powi(3);
    let m = 1024f64.powi(2);
    if b >= g { (g, t(lang, "unit.GB", &[]), 2) } else if b >= m { (m, t(lang, "unit.MB", &[]), 1) } else { (1024.0, t(lang, "unit.KB", &[]), 0) }
}

pub fn size(lang: &str, b: u64) -> String {
    let b = b as f64;
    let (d, u, p) = size_unit(lang, b);
    let v = b / d;
    format!("{} {u}", nf(lang, if v >= 100.0 { 0 } else { p }, v))
}

pub fn pair(lang: &str, a: u64, b: u64) -> String {
    let (d, u, p) = size_unit(lang, b as f64);
    let f = |x: u64| {
        let v = x as f64 / d;
        nf(lang, if v >= 100.0 { 0 } else { p }, v)
    };
    t(lang, "size.of", &[("a", &f(a)), ("b", &f(b)), ("u", &u)])
}

pub fn speed(lang: &str, bps: f64) -> String {
    size(lang, bps.max(0.0) as u64) + &t(lang, "unit.perSec", &[])
}

pub fn time(s: f64) -> String {
    if !s.is_finite() {
        return String::new();
    }
    let s = s.max(0.0).round() as u64;
    let (h, m, ss) = (s / 3600, s % 3600 / 60, s % 60);
    if h > 0 { format!("{h}:{m:02}:{ss:02}") } else { format!("{m}:{ss:02}") }
}

pub fn time_opt(s: Option<f64>) -> String {
    s.map(time).unwrap_or_default()
}

pub fn views(lang: &str, n: Option<u64>) -> String {
    let Some(n) = n else { return String::new() };
    let v = icu::number(lang, "compact-short .#", n as f64).unwrap_or_else(|| compact_fallback(n));
    t(lang, "views", &[("n", &n.to_string()), ("v", &v)])
}

fn compact_fallback(n: u64) -> String {
    let f = |x: f64, s: &str| {
        let r = (x * 10.0).round() / 10.0;
        if r.fract() == 0.0 { format!("{r:.0}{s}") } else { format!("{r:.1}{s}") }
    };
    match n {
        0..=999 => n.to_string(),
        1000..=999_999 => f(n as f64 / 1e3, "K"),
        1_000_000..=999_999_999 => f(n as f64 / 1e6, "M"),
        _ => f(n as f64 / 1e9, "B"),
    }
}

/// YYYYMMDD -> «5 янв. 2024 г.» (день, короткий месяц, год)
pub fn date(lang: &str, d: Option<&str>) -> String {
    let Some(d) = d.filter(|d| d.len() == 8 && d.bytes().all(|c| c.is_ascii_digit())) else { return String::new() };
    let (y, m, dd): (i64, i64, i64) = (d[..4].parse().unwrap(), d[4..6].parse().unwrap(), d[6..].parse().unwrap());
    let ms = days_from_civil(y, m, dd) as f64 * 86_400_000.0;
    icu::date(lang, "yMMMd", ms, true).unwrap_or_else(|| format!("{y}-{m:02}-{dd:02}"))
}

/// Дата и время (dateStyle medium, timeStyle short) по секундам Unix.
pub fn datetime(lang: &str, secs: f64) -> String {
    icu::date(lang, "yMMMdjmm", secs * 1000.0, false).unwrap_or_else(|| {
        let days = (secs / 86400.0).floor() as i64;
        let (y, m, d) = civil_from_days(days);
        format!("{y}-{m:02}-{d:02}")
    })
}

fn days_from_civil(y: i64, m: i64, d: i64) -> i64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let doy = (153 * (if m > 2 { m - 3 } else { m + 9 }) + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146097 + doe - 719468
}

fn civil_from_days(z: i64) -> (i64, i64, i64) {
    let z = z + 719468;
    let era = if z >= 0 { z } else { z - 146096 } / 146097;
    let doe = z - era * 146097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    (if m <= 2 { y + 1 } else { y }, m, d)
}

/// 1080p60 / 2160p
pub fn q_tag(h: u32, fps: Option<f64>) -> String {
    if h == 0 {
        return String::new();
    }
    match fps {
        Some(f) if f > 31.0 => format!("{h}p{}", f.round() as u64),
        _ => format!("{h}p"),
    }
}

/// Целое с разделителями разрядов (toLocaleString).
pub fn group_num(lang: &str, n: u64) -> String {
    icu::number(lang, "precision-integer", n as f64).unwrap_or_else(|| n.to_string())
}

/// «1:20», «1:02:03», «75», «12,5» -> секунды. None — пусто, Some(NaN) — ошибка.
pub fn parse_time(v: &str) -> Option<f64> {
    let v = v.trim().replace(',', ".");
    if v.is_empty() {
        return None;
    }
    let (main, frac) = match v.split_once('.') {
        Some((a, b)) => (a.to_string(), Some(b.to_string())),
        None => (v.clone(), None),
    };
    let parts: Vec<&str> = main.split(':').collect();
    let ok = !parts.is_empty()
        && parts.len() <= 3
        && !parts[0].is_empty()
        && parts[0].bytes().all(|c| c.is_ascii_digit())
        && parts[1..].iter().all(|p| (1..=2).contains(&p.len()) && p.bytes().all(|c| c.is_ascii_digit()))
        && frac.as_ref().is_none_or(|f| !f.is_empty() && f.bytes().all(|c| c.is_ascii_digit()));
    if !ok {
        return Some(f64::NAN);
    }
    let mut total = 0f64;
    let n = parts.len();
    for (i, p) in parts.iter().enumerate() {
        let mut x: f64 = p.parse().unwrap_or(0.0);
        if i == n - 1 && let Some(f) = &frac {
            x += format!("0.{f}").parse::<f64>().unwrap_or(0.0);
        }
        total = total * 60.0 + x;
    }
    Some(total)
}

// ---------------------------------------------------------------- ширина текста

struct Font {
    data: Vec<u8>,
}

fn sans_font(bold: bool) -> Option<&'static Font> {
    static F: OnceLock<Option<Font>> = OnceLock::new();
    static B: OnceLock<Option<Font>> = OnceLock::new();
    // окно рисует Segoe UI Variable (если есть) — мерить тем же файлом
    let names: &[&str] = if bold { &["SegUIVar.ttf", "seguisb.ttf", "segoeui.ttf"] } else { &["SegUIVar.ttf", "segoeui.ttf", "arial.ttf"] };
    (if bold { &B } else { &F }).get_or_init(|| {
        let dir = std::env::var_os("WINDIR").map(std::path::PathBuf::from).unwrap_or_else(|| "C:\\Windows".into());
        for name in names {
            if let Ok(data) = std::fs::read(dir.join("Fonts").join(name)) {
                return Some(Font { data });
            }
        }
        None
    })
    .as_ref()
}

/// Ширина строки шрифтом Segoe UI (px); моноширинный — 0,55 em на знак.
pub fn text_width(s: &str, px: f32, mono: bool) -> f32 {
    if mono {
        return s.chars().map(|c| if (c as u32) > 0x2E80 { px } else { px * 0.55 }).sum();
    }
    text_width_w(s, px, false)
}

/// Ширина строки шрифтом Segoe UI (bold — Semibold).
pub fn text_width_w(s: &str, px: f32, bold: bool) -> f32 {
    use skrifa::instance::Size;
    use skrifa::{FontRef, MetadataProvider};
    let Some(f) = sans_font(bold) else { return s.chars().count() as f32 * px * 0.5 };
    let Ok(font) = FontRef::new(&f.data) else { return s.chars().count() as f32 * px * 0.5 };
    let cm = font.charmap();
    let loc = font.axes().location([("wght", if bold { 600.0 } else { 400.0 }), ("opsz", 10.5)]);
    let gm = font.glyph_metrics(Size::new(px), &loc);
    s.chars()
        .map(|c| cm.map(c).and_then(|g| gm.advance_width(g)).unwrap_or(if (c as u32) > 0x2E80 { px } else { px * 0.55 }))
        .sum()
}

/// Обрезка середины: диск и хвост имени файла остаются видны (fitMiddle).
pub fn fit_middle(full: &str, width: f32, px: f32, mono: bool) -> String {
    if width <= 0.0 || text_width(full, px, mono) <= width {
        return full.to_string();
    }
    let chars: Vec<char> = full.chars().collect();
    let (mut lo, mut hi) = (4usize, chars.len());
    let mut best: String = chars.iter().take(3).collect::<String>() + "…";
    while lo <= hi {
        let keep = (lo + hi) / 2;
        let head = ((keep as f32) * 0.45).ceil() as usize;
        let tail = keep - head;
        let s: String = chars[..head].iter().collect::<String>() + "…" + &chars[chars.len() - tail..].iter().collect::<String>();
        if text_width(&s, px, mono) <= width {
            best = s;
            lo = keep + 1;
        } else {
            if keep == 0 { break; }
            hi = keep - 1;
        }
    }
    best
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn parse() {
        assert_eq!(parse_time(""), None);
        assert_eq!(parse_time("1:20"), Some(80.0));
        assert_eq!(parse_time("1:02:03"), Some(3723.0));
        assert_eq!(parse_time("12,5"), Some(12.5));
        assert!(parse_time("1:2:3:4").unwrap().is_nan());
        assert!(parse_time("abc").unwrap().is_nan());
        assert!(parse_time("1:200").unwrap().is_nan());
    }
    #[test]
    fn times() {
        assert_eq!(time(65.0), "1:05");
        assert_eq!(time(3725.0), "1:02:05");
        assert_eq!(q_tag(1080, Some(60.0)), "1080p60");
        assert_eq!(q_tag(1080, Some(30.0)), "1080p");
        assert_eq!(days_from_civil(1970, 1, 1), 0);
        assert_eq!(civil_from_days(19723), (2024, 1, 1));
    }
}
