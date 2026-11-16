//! Вход на сайты, присланный расширением, — между запусками программы.
//! Файл logins.dat в папке данных зашифрован Windows (DPAPI): прочитать его можно только
//! под той же учётной записью Windows. Внутри — JSON {сайт: cookies.txt}.
use std::collections::HashMap;
use std::path::Path;

pub const FILE: &str = "logins.dat";

pub fn load(path: &Path) -> HashMap<String, String> {
    let Ok(enc) = std::fs::read(path) else { return HashMap::new() };
    let Some(plain) = dpapi(&enc, false) else { return HashMap::new() };
    let mut m: HashMap<String, String> = serde_json::from_slice(&plain).unwrap_or_default();
    m.retain(|_, v| has_live_cookie(v));
    m
}

pub fn save(path: &Path, m: &HashMap<String, String>) {
    let Ok(plain) = serde_json::to_vec(m) else { return };
    let Some(enc) = dpapi(&plain, true) else { return };
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    // сначала во временный файл: обрыв записи не должен портить сохранённый вход
    let tmp = path.with_extension("tmp");
    if std::fs::write(&tmp, enc).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

/// Есть ли в cookies.txt хоть одна ещё не истёкшая запись (0 — cookie сеанса, считаем живой).
fn has_live_cookie(netscape: &str) -> bool {
    let now = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    netscape.lines().filter(|l| !l.starts_with('#') && !l.trim().is_empty()).any(|l| {
        let exp: u64 = l.split('\t').nth(4).and_then(|e| e.trim().parse().ok()).unwrap_or(0);
        exp == 0 || exp > now
    })
}

#[cfg(windows)]
fn dpapi(data: &[u8], protect: bool) -> Option<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let input = CRYPT_INTEGER_BLOB { cbData: u32::try_from(data.len()).ok()?, pbData: data.as_ptr() as *mut u8 };
    let mut out = CRYPT_INTEGER_BLOB { cbData: 0, pbData: std::ptr::null_mut() };
    // SAFETY: входной буфер живёт до конца вызова; выходной выделяет Windows, освобождаем LocalFree.
    unsafe {
        let ok = if protect {
            CryptProtectData(&input, std::ptr::null(), std::ptr::null(), std::ptr::null(), std::ptr::null(),
                             CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        } else {
            CryptUnprotectData(&input, std::ptr::null_mut(), std::ptr::null(), std::ptr::null(), std::ptr::null(),
                               CRYPTPROTECT_UI_FORBIDDEN, &mut out)
        };
        if ok == 0 || out.pbData.is_null() {
            return None;
        }
        let v = std::slice::from_raw_parts(out.pbData, out.cbData as usize).to_vec();
        LocalFree(out.pbData as _);
        Some(v)
    }
}

#[cfg(not(windows))]
fn dpapi(_: &[u8], _: bool) -> Option<Vec<u8>> { None }

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn roundtrip_encrypted() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        let mut m = HashMap::new();
        m.insert("youtube.com".to_string(), "# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t0\tSID\tsecret\n".to_string());
        save(&p, &m);
        let raw = std::fs::read(&p).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("secret"), "файл должен быть зашифрован");
        assert_eq!(load(&p), m);
    }

    #[test]
    fn expired_dropped() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join(FILE);
        let mut m = HashMap::new();
        m.insert("old.com".to_string(), ".old.com\tTRUE\t/\tTRUE\t5\ta\tb\n".to_string());
        save(&p, &m);
        assert!(load(&p).is_empty());
    }
}
