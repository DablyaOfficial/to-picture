//! Понятные сообщения об ошибках yt-dlp (ERROR_HINTS), признаки «нужен вход» и «yt-dlp устарел».
use crate::i18n::tr;
use regex::Regex;
use std::sync::OnceLock;

const ERROR_HINTS: &[(&str, &str)] = &[
    (r"could not copy .*cookie database", "py.err.cookieLocked"),
    (r"failed to decrypt|app.bound|dpapi", "py.err.cookieDecrypt"),
    (r"could not find .*cookies database|could not find firefox", "py.err.cookieNoBrowser"),
    (r"cookies? file .*(not exist|no such)|\[errno 2\].*cookies", "py.err.cookieNoFile"),
    (r"unable to connect to proxy|proxyerror|socks|tunnel connection failed", "py.err.proxy"),
    (r"private video", "py.err.private"),
    (r"sign in to confirm your age|age.restricted", "py.err.age"),
    (r"not a bot", "py.err.bot"),
    (r"members.only|join this channel", "py.err.members"),
    (concat!(r"authentication is required|only works when logged.in|login required|requires? (a )?log.?in|",
             r"logged.in|use --cookies|account credentials"), "py.err.login"),
    (r"http error 404|: not found\.|video (is )?unavailable|is not available|has been removed|does not exist",
     "py.err.unavailable"),
    (r"no video could be found|no video formats found|there is no video|no media found", "py.err.noVideo"),
    (r"requested format is not available", "py.err.noFormat"),
    (r"is not a valid url|unsupported url", "py.err.badUrl"),
    (r"premieres in|live event will begin", "py.err.premiere"),
    (r"http error 403", "py.err.http403"),
    (r"getaddrinfo|timed out|connection (refused|reset|aborted)|network is unreachable|unable to download webpage",
     "py.err.network"),
];

fn hints() -> &'static [(Regex, &'static str)] {
    static H: OnceLock<Vec<(Regex, &'static str)>> = OnceLock::new();
    // в Python шаблоны применяются к тексту в нижнем регистре; (?s) не нужен — строки однострочные,
    // но . в Python не совпадает с \n, как и здесь
    H.get_or_init(|| ERROR_HINTS.iter().map(|(p, k)| (Regex::new(p).expect("hint"), *k)).collect())
}

/// Ошибки, которые обычно лечатся обновлением yt-dlp.
pub fn outdated_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(concat!(r"(?i)http error 403|nsig|signature|not a bot|unable to extract|",
                           r"requested format is not available|precondition|player response")).unwrap()
    })
}

/// Сайт требует вход.
pub fn login_re() -> &'static Regex {
    static R: OnceLock<Regex> = OnceLock::new();
    R.get_or_init(|| {
        Regex::new(concat!(r"(?i)private video|sign in to confirm|age.restricted|not a bot|members.only|join this channel|",
                           r"authentication is required|logged.in|login required|requires? (a )?log.?in|use --cookies|",
                           r"account credentials")).unwrap()
    })
}

pub fn http403(raw: &str) -> bool {
    raw.to_lowercase().contains("http error 403")
}

/// Строка ERROR yt-dlp -> понятный текст на языке интерфейса.
pub fn friendly_error(text: &str, lang: &str) -> String {
    let low = text.to_lowercase();
    for (re, key) in hints() {
        if re.is_match(&low) {
            return tr(lang, key, &[]);
        }
    }
    static STRIP: OnceLock<Regex> = OnceLock::new();
    let strip = STRIP.get_or_init(|| Regex::new(r"^ERROR:\s*(\[[^\]]+\]\s*)?([\w-]+:\s*)?").unwrap());
    let s = strip.replace(text, "");
    let s = s.trim();
    if s.is_empty() { tr(lang, "py.err.unknown", &[]) } else { s.to_string() }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn friendly() {
        let en = |k: &str| tr("en", k, &[]);
        assert_eq!(friendly_error("ERROR: [youtube] abc: Private video. Sign in if you've been granted access", "en"), en("py.err.private"));
        assert_eq!(friendly_error("ERROR: [youtube] x: Sign in to confirm you’re not a bot", "en"), en("py.err.bot"));
        assert_eq!(friendly_error("ERROR: [youtube] x: Sign in to confirm your age", "en"), en("py.err.age"));
        assert_eq!(friendly_error("ERROR: Unsupported URL: https://example.com/", "en"), en("py.err.badUrl"));
        assert_eq!(friendly_error("ERROR: unable to download video data: HTTP Error 403: Forbidden", "en"), en("py.err.http403"));
        assert_eq!(friendly_error("ERROR: [Errno 2] No such file: cookies.txt", "en"), en("py.err.cookieNoFile"));
        assert_eq!(friendly_error("ERROR: [generic] 'x' is not a valid URL", "en"), en("py.err.badUrl"));
        assert_eq!(friendly_error("ERROR: [instagram] abc: Requested content is only available for registered users, use --cookies", "en"), en("py.err.login"));
        assert_eq!(friendly_error("ERROR: [vimeo] 123: Something odd happened", "en"), "Something odd happened");
        assert_eq!(friendly_error("ERROR: Weird", "en"), "Weird");
        assert_eq!(friendly_error("ERROR: ", "en"), en("py.err.unknown"));
        assert_eq!(friendly_error("", "ru"), tr("ru", "py.err.unknown", &[]));
        assert_ne!(tr("ru", "py.err.private", &[]), en("py.err.private"));
        assert!(login_re().is_match("ERROR: Join this channel to get access"));
        assert!(!login_re().is_match("ERROR: HTTP Error 404"));
        assert!(outdated_re().is_match("ERROR: nsig extraction failed"));
        assert!(http403("x HTTP Error 403: Forbidden"));
    }
}
