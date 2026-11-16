//! Вход на сайты (cookies) и прокси — для каждого вызова yt-dlp (net_args в Python).
use crate::Settings;
use std::path::Path;

/// Выбран вход через файл cookies.txt, а файла нет.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CookiesMissing;

/// browser — cookies.txt, присланный расширением (используется, только если вход в параметрах не выбран).
pub fn net_args(o: &Settings, tmpdir: Option<&Path>, browser: Option<&str>) -> Result<Vec<String>, CookiesMissing> {
    let mut a: Vec<String> = vec![];
    let mode = o.cookies.as_str();
    match (browser.filter(|b| !b.is_empty()), tmpdir) {
        (Some(b), Some(tmp)) if mode == "off" || mode.is_empty() => {
            let f = tmp.join("browser-cookies.txt");
            // ошибка записи в Python — исключение; здесь просто без cookies нельзя, поэтому передаём путь всё равно
            let _ = std::fs::write(&f, b);
            a.extend(["--cookies".into(), f.to_string_lossy().into_owned()]);
        }
        _ if mode == "firefox" => a.extend(["--cookies-from-browser".into(), "firefox".into()]),
        _ if mode == "file" => {
            let mut src = std::path::PathBuf::from(&o.cookies_file);
            if o.cookies_file.is_empty() || !src.is_file() {
                // без этой проверки yt-dlp падает с трассировкой вместо понятной ошибки
                return Err(CookiesMissing);
            }
            // yt-dlp дописывает cookies обратно в файл — даём ему копию, файл пользователя не трогаем
            if let Some(tmp) = tmpdir {
                let dst = tmp.join("cookies.txt");
                if std::fs::copy(&src, &dst).is_ok() {
                    src = dst;
                }
            }
            a.extend(["--cookies".into(), src.to_string_lossy().into_owned()]);
        }
        _ => {}
    }
    let proxy = o.proxy.trim();
    if !proxy.is_empty() {
        a.extend(["--proxy".into(), proxy.to_string()]);
    }
    Ok(a)
}

/// Список cookies от расширения (JSON chrome.cookies) -> cookies.txt, который понимает yt-dlp.
pub fn netscape_cookies(cookies: &[serde_json::Value]) -> String {
    let mut lines = vec!["# Netscape HTTP Cookie File".to_string()];
    for c in cookies {
        let (Some(dom), Some(name), Some(value)) = (
            c.get("domain").and_then(|v| v.as_str()),
            c.get("name").and_then(|v| v.as_str()),
            c.get("value").and_then(|v| v.as_str()),
        ) else { continue };
        let b = |k: &str| c.get(k).and_then(|v| v.as_bool()).unwrap_or(false);
        let sub = !b("hostOnly");
        let dom = if sub && !dom.starts_with('.') { format!(".{dom}") } else { dom.to_string() };
        let exp = c.get("expirationDate").and_then(|v| v.as_f64()).unwrap_or(0.0) as i64;
        lines.push([
            format!("{}{dom}", if b("httpOnly") { "#HttpOnly_" } else { "" }),
            (if sub { "TRUE" } else { "FALSE" }).into(),
            c.get("path").and_then(|v| v.as_str()).filter(|p| !p.is_empty()).unwrap_or("/").into(),
            (if b("secure") { "TRUE" } else { "FALSE" }).into(),
            exp.to_string(),
            name.into(),
            value.into(),
        ].join("\t"));
    }
    lines.join("\n") + "\n"
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn args() {
        let d = tempfile::tempdir().unwrap();
        let mut s = Settings::default();
        assert!(net_args(&s, Some(d.path()), None).unwrap().is_empty());
        s.proxy = "  socks5://127.0.0.1:1080 ".into();
        assert_eq!(net_args(&s, None, None).unwrap(), ["--proxy", "socks5://127.0.0.1:1080"]);
        s.proxy.clear();
        // cookies от расширения — только если вход в параметрах выключен и есть временная папка
        let a = net_args(&s, Some(d.path()), Some("# Netscape\n")).unwrap();
        assert_eq!(a[0], "--cookies");
        assert!(a[1].ends_with("browser-cookies.txt"));
        assert_eq!(std::fs::read_to_string(&a[1]).unwrap(), "# Netscape\n");
        assert!(net_args(&s, None, Some("x")).unwrap().is_empty());
        s.cookies = "firefox".into();
        assert_eq!(net_args(&s, Some(d.path()), Some("x")).unwrap(), ["--cookies-from-browser", "firefox"]);
        s.cookies = "file".into();
        assert_eq!(net_args(&s, Some(d.path()), None), Err(CookiesMissing));
        let f = d.path().join("my.txt");
        std::fs::write(&f, "c").unwrap();
        s.cookies_file = f.to_string_lossy().into();
        let sub = d.path().join("t");
        std::fs::create_dir(&sub).unwrap();
        let a = net_args(&s, Some(&sub), None).unwrap();
        assert_eq!(a, ["--cookies".to_string(), sub.join("cookies.txt").to_string_lossy().into_owned()]);
        assert_eq!(net_args(&s, None, None).unwrap()[1], s.cookies_file);
    }

    #[test]
    fn netscape() {
        let c: Vec<serde_json::Value> = serde_json::from_str(r#"[
            {"domain": "youtube.com", "hostOnly": false, "httpOnly": true, "path": "/", "secure": true, "expirationDate": 1999999999.5, "name": "SID", "value": "a"},
            {"domain": "www.youtube.com", "hostOnly": true, "name": "P", "value": "b"},
            {"name": "bad"}]"#).unwrap();
        assert_eq!(netscape_cookies(&c), "# Netscape HTTP Cookie File\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1999999999\tSID\ta\nwww.youtube.com\tFALSE\t/\tFALSE\t0\tP\tb\n");
    }
}
