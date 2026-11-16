//! Переводы: ui/locales/<код>.json, встроенные в exe. Те же файлы, что у Python-версии.
//! Значение — строка с {параметрами} или объект форм множественного числа {one, few, many, other}.
use serde_json::{Map, Value};
use std::collections::HashMap;
use std::sync::OnceLock;

const RAW: &[(&str, &str)] = &[
    ("de", include_str!("../../../ui/locales/de.json")),
    ("en", include_str!("../../../ui/locales/en.json")),
    ("es", include_str!("../../../ui/locales/es.json")),
    ("fr", include_str!("../../../ui/locales/fr.json")),
    ("ja", include_str!("../../../ui/locales/ja.json")),
    ("ko", include_str!("../../../ui/locales/ko.json")),
    ("pt-BR", include_str!("../../../ui/locales/pt-BR.json")),
    ("ru", include_str!("../../../ui/locales/ru.json")),
    ("tr", include_str!("../../../ui/locales/tr.json")),
    ("zh-CN", include_str!("../../../ui/locales/zh-CN.json")),
];

fn all() -> &'static HashMap<&'static str, Map<String, Value>> {
    static L: OnceLock<HashMap<&'static str, Map<String, Value>>> = OnceLock::new();
    L.get_or_init(|| {
        RAW.iter()
            .map(|(code, text)| (*code, serde_json::from_str::<Map<String, Value>>(text).unwrap_or_default()))
            .collect()
    })
}

#[derive(Debug, Clone)]
pub struct Lang { pub code: String, pub name: String }

/// Все переведённые языки: код и самоназвание (_name).
pub fn available_langs() -> Vec<Lang> {
    RAW.iter()
        .map(|(c, _)| Lang {
            code: c.to_string(),
            name: all()[c].get("_name").and_then(Value::as_str).unwrap_or(c).to_string(),
        })
        .collect()
}

/// Есть ли у языка признак «_lower» (подписи пишутся со строчной буквы) и т.п. служебные поля.
pub fn meta(lang: &str, key: &str) -> Option<&'static Value> {
    all().get(lang).and_then(|m| m.get(key))
}

/// Язык интерфейса Windows, сведённый к одному из переведённых.
pub fn system_lang() -> String {
    let name = windows_ui_locale().unwrap_or_else(|| "en-US".into());
    let codes: Vec<&str> = RAW.iter().map(|(c, _)| *c).collect();
    if codes.contains(&name.as_str()) {
        return name;
    }
    let base = name.split('-').next().unwrap_or("").to_ascii_lowercase();
    codes.iter().find(|c| c.split('-').next().unwrap_or("").eq_ignore_ascii_case(&base))
        .map(|c| c.to_string())
        .unwrap_or_else(|| "en".into())
}

fn windows_ui_locale() -> Option<String> {
    use windows_sys::Win32::Globalization::{GetUserDefaultUILanguage, LCIDToLocaleName};
    let mut buf = [0u16; 85];
    let n = unsafe { LCIDToLocaleName(GetUserDefaultUILanguage() as u32, buf.as_mut_ptr(), buf.len() as i32, 0) };
    (n > 1).then(|| String::from_utf16_lossy(&buf[..(n - 1) as usize]))
}

/// Настройка lang («auto» или код) -> код переведённого языка.
pub fn resolve_lang(setting: &str) -> String {
    if RAW.iter().any(|(c, _)| *c == setting) { setting.to_string() } else { system_lang() }
}

/// Сырой элемент перевода (строка или объект форм) с откатом на английский.
fn raw(lang: &str, key: &str) -> Option<&'static Value> {
    all().get(lang).and_then(|m| m.get(key)).or_else(|| all().get("en").and_then(|m| m.get(key)))
}

/// Категория множественного числа (CLDR, для переведённых языков).
pub fn plural_category(lang: &str, n: f64) -> &'static str {
    let int = n.fract() == 0.0 && n >= 0.0;
    let i = n.abs().trunc() as u64;
    match lang.split('-').next().unwrap_or("") {
        "ja" | "ko" | "zh" => "other",
        "ru" => {
            if !int { return "other"; }
            let (m10, m100) = (i % 10, i % 100);
            if m10 == 1 && m100 != 11 { "one" }
            else if (2..=4).contains(&m10) && !(12..=14).contains(&m100) { "few" }
            else { "many" }
        }
        "fr" | "pt" => if i <= 1 { "one" } else { "other" },
        _ => if int && i == 1 { "one" } else { "other" },
    }
}

/// Перевод с подстановкой {параметров}. Если среди параметров есть n, выбирается форма множественного числа
/// (как t() в index.html: при n ≥ 1000 и наличии формы many — many, «2,4 млн просмотров»).
pub fn tr(lang: &str, key: &str, params: &[(&str, &str)]) -> String {
    let s = match raw(lang, key) {
        None => key.to_string(),
        Some(Value::String(s)) => s.clone(),
        Some(Value::Object(forms)) => {
            let n: f64 = params.iter().find(|(k, _)| *k == "n").and_then(|(_, v)| v.replace(',', ".").parse().ok()).unwrap_or(0.0);
            let cat = if n >= 1000.0 && forms.contains_key("many") { "many" } else { plural_category(lang, n) };
            forms.get(cat).or_else(|| forms.get("other")).or_else(|| forms.values().next())
                .and_then(Value::as_str).unwrap_or(key).to_string()
        }
        Some(v) => v.to_string(),
    };
    fill(&s, params)
}

/// Подставить {k} -> v; неизвестные параметры становятся пустой строкой (как в JS).
pub fn fill(s: &str, params: &[(&str, &str)]) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(a) = rest.find('{') {
        out.push_str(&rest[..a]);
        let tail = &rest[a + 1..];
        match tail.find('}') {
            Some(b) if tail[..b].chars().all(|c| c.is_ascii_alphanumeric() || c == '_') && b > 0 => {
                let k = &tail[..b];
                out.push_str(params.iter().find(|(pk, _)| *pk == k).map(|(_, v)| *v).unwrap_or(""));
                rest = &tail[b + 1..];
            }
            _ => { out.push('{'); rest = tail; }
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn all_locales_parse_and_match_en() {
        let en = &all()["en"];
        assert!(en.len() > 300);
        for (c, _) in RAW { assert!(!all()[c].is_empty(), "{c}"); }
    }
    #[test]
    fn plurals() {
        assert_eq!(plural_category("ru", 1.0), "one");
        assert_eq!(plural_category("ru", 3.0), "few");
        assert_eq!(plural_category("ru", 11.0), "many");
        assert_eq!(plural_category("en", 1.0), "one");
        assert_eq!(plural_category("fr", 0.0), "one");
        assert_eq!(fill("{a} и {b}", &[("a", "x")]), "x и ");
    }
}
