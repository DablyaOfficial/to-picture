//! To Picture — связь с расширением для браузера (native messaging), inbox и пробуждение окна.
//! Порт раздела «Bridge» из ToPicture.pyw. Формат файлов (native-host.json, inbox/*.json, ipc.json)
//! и разделы реестра — те же, что у Python-версии.
//!
//! КОНТРАКТ: публичные сигнатуры этого файла использует crate app. Только добавлять, не менять.

use std::io::{Read, Write};
use std::net::{Ipv4Addr, SocketAddr, TcpListener, TcpStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};
use topicture_core::paths;
use topicture_core::settings::{load_state, save_state};

pub const HOST_NAME: &str = "com.topicture.app";
/// Постоянный ID: задан ключом в extension/manifest.json.
pub const EXT_ID: &str = "kefllcapjmcgojghjpaofcpoigeiocgi";

/// Браузеры на Chromium ищут описание связи каждый в своём разделе реестра (HKCU).
pub const HOST_KEYS: &[&str] = &[
    r"Software\Google\Chrome\NativeMessagingHosts",
    r"Software\Microsoft\Edge\NativeMessagingHosts",
    r"Software\Chromium\NativeMessagingHosts",
    r"Software\BraveSoftware\Brave-Browser\NativeMessagingHosts",
    r"Software\Vivaldi\NativeMessagingHosts",
    r"Software\Yandex\YandexBrowser\NativeMessagingHosts",
];

/// Ссылки старше этого (секунды) из inbox отбрасываются.
pub const INBOX_MAX_AGE: f64 = 3600.0;

/// Ссылка, присланная расширением.
#[derive(Debug, Clone)]
pub struct InboxLink {
    pub url: String,
    pub title: String,
    /// cookies сайта в формате cookies.txt (Netscape); None — расширение не прислало
    pub cookies: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ExtStatus {
    /// связь в реестре есть и ведёт к этому exe
    pub registered: bool,
    pub folder: String,
    pub has_folder: bool,
    /// расширением уже пользовались (ссылка приходила)
    pub seen: bool,
    pub portable: bool,
    pub id: String,
}

// ---------------------------------------------------------------- пути

/// %DATA%\inbox — сюда посредник кладёт присланные ссылки.
pub fn inbox_dir() -> PathBuf { paths::data_dir().join("inbox") }
/// %DATA%\ipc.json — {port, token, pid} открытого окна.
pub fn ipc_file() -> PathBuf { paths::data_dir().join("ipc.json") }
/// %DATA%\native-host.json — описание связи для браузера.
pub fn host_manifest() -> PathBuf { paths::data_dir().join("native-host.json") }

/// Что запускает браузер: сам exe (и при разработке тоже — bat не нужен).
pub fn host_executable() -> PathBuf {
    std::env::current_exe().map(|p| strip_unc(&p)).unwrap_or_default()
}

fn strip_unc(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s).to_string())
}

/// Сравнение путей Windows: без учёта регистра, \ и / одинаковы, без префикса \\?\.
fn norm_path(p: &str) -> String {
    let s = p.strip_prefix(r"\\?\").unwrap_or(p);
    s.replace('/', "\\").trim_end_matches('\\').to_lowercase()
}

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

/// Истинность значения по правилам Python (None, False, 0, "", [], {} — ложь).
fn truthy(v: Option<&Value>) -> bool {
    match v {
        None | Some(Value::Null) => false,
        Some(Value::Bool(b)) => *b,
        Some(Value::Number(n)) => n.as_f64().is_some_and(|f| f != 0.0),
        Some(Value::String(s)) => !s.is_empty(),
        Some(Value::Array(a)) => !a.is_empty(),
        Some(Value::Object(o)) => !o.is_empty(),
    }
}

/// `m.get(key) or ""` для строковых полей.
fn str_or_empty(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        _ => String::new(),
    }
}

// ---------------------------------------------------------------- регистрация

/// Аргументы, с которыми браузер запускает программу как native host (первый — chrome-extension://…).
pub fn is_host_invocation(args: &[String]) -> bool {
    args.iter().any(|a| a.starts_with("chrome-extension://"))
}

/// Записать native-host.json и разделы реестра (HKCU) для всех браузеров на Chromium.
pub fn register_extension() -> std::io::Result<()> {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let manifest = host_manifest();
    std::fs::create_dir_all(paths::data_dir())?;
    let body = json!({
        "name": HOST_NAME, "description": "To Picture",
        "path": host_executable().to_string_lossy(), "type": "stdio",
        "allowed_origins": [format!("chrome-extension://{EXT_ID}/")],
    });
    std::fs::write(&manifest, serde_json::to_string_pretty(&body)?)?;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    let value = manifest.to_string_lossy().to_string();
    for key in HOST_KEYS {
        let (k, _) = hkcu.create_subkey(format!(r"{key}\{HOST_NAME}"))?;
        k.set_value("", &value)?;
    }
    Ok(())
}

pub fn unregister_extension() {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
    for key in HOST_KEYS {
        let _ = hkcu.delete_subkey(format!(r"{key}\{HOST_NAME}"));
    }
    let _ = std::fs::remove_file(host_manifest());
    let _ = std::fs::remove_file(paths::data_dir().join("native-host.bat")); // от Python-версии при разработке
}

/// Связь настроена и ведёт именно к этой копии программы.
pub fn extension_registered() -> bool {
    use winreg::RegKey;
    use winreg::enums::HKEY_CURRENT_USER;
    let Ok(k) = RegKey::predef(HKEY_CURRENT_USER).open_subkey(format!(r"{}\{HOST_NAME}", HOST_KEYS[0])) else {
        return false;
    };
    let Ok(manifest) = k.get_value::<String, _>("") else { return false };
    let Ok(text) = std::fs::read_to_string(&manifest) else { return false };
    let Ok(v) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { return false };
    let target = v.get("path").and_then(Value::as_str).unwrap_or("");
    !target.is_empty() && norm_path(target) == norm_path(&host_executable().to_string_lossy())
}

pub fn ext_status() -> ExtStatus {
    let st = load_state();
    let dir = paths::extension_dir();
    ExtStatus {
        registered: extension_registered(),
        folder: dir.to_string_lossy().to_string(),
        has_folder: dir.is_dir(),
        seen: truthy(st.get("ext_seen")),
        portable: paths::portable(),
        id: EXT_ID.to_string(),
    }
}

/// Показать один раз панель подключения расширения: связь есть, им не пользовались, панель не показывали.
/// Сам ставит отметку ext_help_shown.
pub fn should_show_help_once() -> bool {
    let mut st = load_state();
    let show = extension_registered()
        && !truthy(st.get("ext_seen"))
        && !truthy(st.get("ext_help_shown"))
        && paths::extension_dir().is_dir();
    if show {
        st.insert("ext_help_shown".into(), Value::Bool(true));
        save_state(&st);
    }
    show
}

// ---------------------------------------------------------------- Netscape cookies

/// Список cookies от расширения (объекты chrome.cookies) -> формат cookies.txt, который понимает yt-dlp.
pub fn netscape_cookies(cookies: &Value) -> String {
    let mut lines = vec!["# Netscape HTTP Cookie File".to_string()];
    if let Value::Array(list) = cookies {
        for c in list {
            if let Some(line) = netscape_line(c) {
                lines.push(line);
            }
        }
    }
    lines.join("\n") + "\n"
}

fn netscape_line(c: &Value) -> Option<String> {
    let c = c.as_object()?;
    let mut dom = c.get("domain")?.as_str()?.to_string();
    let sub = !truthy(c.get("hostOnly"));
    if sub && !dom.starts_with('.') {
        dom.insert(0, '.');
    }
    let path = match c.get("path") {
        Some(v) if truthy(Some(v)) => v.as_str()?.to_string(),
        _ => "/".to_string(),
    };
    let exp: i64 = match c.get("expirationDate") {
        Some(v) if truthy(Some(v)) => match v {
            Value::Number(n) => n.as_i64().or_else(|| n.as_f64().map(|f| f.trunc() as i64))?,
            Value::Bool(true) => 1,
            Value::String(s) => s.trim().parse().ok()?,
            _ => return None,
        },
        _ => 0,
    };
    let name = c.get("name")?.as_str()?;
    let value = c.get("value")?.as_str()?;
    let prefix = if truthy(c.get("httpOnly")) { "#HttpOnly_" } else { "" };
    Some([
        format!("{prefix}{dom}"),
        (if sub { "TRUE" } else { "FALSE" }).to_string(),
        path,
        (if truthy(c.get("secure")) { "TRUE" } else { "FALSE" }).to_string(),
        exp.to_string(),
        name.to_string(),
        value.to_string(),
    ]
    .join("\t"))
}

// ---------------------------------------------------------------- сторона браузера (native host)

/// Ответ посредника на одно сообщение расширения.
pub fn host_message(m: &Value) -> Value {
    let Some(obj) = m.as_object() else {
        return json!({"ok": false, "error": "bad message"});
    };
    match parse_message(obj) {
        Message::Ping => json!({"ok": true, "version": topicture_core::APP_VERSION}),
        Message::Bad => json!({"ok": false, "error": "bad url"}),
        Message::Add { url, title, cookies } => {
            if let Err(e) = write_inbox(&inbox_dir(), &url, &title, cookies) {
                return json!({"ok": false, "error": e.to_string()});
            }
            let mut st = load_state();
            st.insert("ext_seen".into(), json!(now()));
            save_state(&st);
            if wake_app() {
                json!({"ok": true, "launched": false})
            } else {
                json!({"ok": launch_app(), "launched": true})
            }
        }
    }
}

/// Разобранное сообщение расширения.
#[derive(Debug, PartialEq)]
pub enum Message {
    Ping,
    Add { url: String, title: String, cookies: Value },
    Bad,
}

pub fn parse_message(m: &Map<String, Value>) -> Message {
    let ty = m.get("type").and_then(Value::as_str);
    if ty == Some("ping") {
        return Message::Ping;
    }
    let url = str_or_empty(m.get("url"));
    if ty != Some("add") || !(url.starts_with("http://") || url.starts_with("https://")) {
        return Message::Bad;
    }
    let cookies = match m.get("cookies") {
        Some(v) if truthy(Some(v)) => v.clone(),
        _ => Value::Array(vec![]),
    };
    Message::Add { url, title: str_or_empty(m.get("title")), cookies }
}

/// Положить ссылку в inbox атомарно (.tmp + переименование), как Python. Возвращает путь файла.
pub fn write_inbox(dir: &Path, url: &str, title: &str, cookies: Value) -> std::io::Result<PathBuf> {
    std::fs::create_dir_all(dir)?;
    let t = now();
    let name = format!("{t:.6}-{}.json", std::process::id());
    let tmp = dir.join(format!("{name}.tmp"));
    let body = json!({"url": url, "title": title, "cookies": cookies, "time": t});
    std::fs::write(&tmp, serde_json::to_string(&body)?)?;
    let dest = dir.join(&name);
    if let Err(e) = std::fs::rename(&tmp, &dest) {
        let _ = std::fs::remove_file(&tmp);
        return Err(e);
    }
    Ok(dest)
}

/// Разбудить открытое окно: оно заберёт ссылки из inbox. false — окна нет.
pub fn wake_app() -> bool {
    let Ok(text) = std::fs::read_to_string(ipc_file()) else { return false };
    let Ok(ipc) = serde_json::from_str::<Value>(text.trim_start_matches('\u{feff}')) else { return false };
    let (Some(port), Some(token)) = (ipc.get("port").and_then(Value::as_u64), ipc.get("token").and_then(Value::as_str))
    else {
        return false;
    };
    let Ok(port) = u16::try_from(port) else { return false };
    let timeout = Duration::from_millis(1500);
    let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, port));
    let Ok(mut s) = TcpStream::connect_timeout(&addr, timeout) else { return false };
    let _ = s.set_read_timeout(Some(timeout));
    let _ = s.set_write_timeout(Some(timeout));
    if s.write_all(format!("{token}\n").as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0u8; 8];
    let mut got = 0;
    // Python: s.recv(8) == b"ok"; дочитываем, пока сервер не закроет соединение
    while got < buf.len() {
        match s.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) => break,
        }
        if &buf[..got] == b"ok" {
            break;
        }
    }
    &buf[..got] == b"ok"
}

/// Запустить программу отдельно от браузера (браузер не закроет её вместе с собой).
pub fn launch_app() -> bool {
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
    let exe = host_executable();
    for extra in [CREATE_BREAKAWAY_FROM_JOB, 0] {
        let spawned = Command::new(&exe)
            .current_dir(paths::install_dir())
            .creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP | extra)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn();
        if spawned.is_ok() {
            return true;
        }
    }
    false
}

/// stdin/stdout как сырые дескрипторы Windows (ReadFile/WriteFile): без перекодировки и буферов консоли.
mod stdio {
    use windows_sys::Win32::Foundation::{HANDLE, INVALID_HANDLE_VALUE};
    use windows_sys::Win32::Storage::FileSystem::{FlushFileBuffers, ReadFile, WriteFile};
    use windows_sys::Win32::System::Console::{GetStdHandle, STD_INPUT_HANDLE, STD_OUTPUT_HANDLE};

    fn handle(which: u32) -> Option<HANDLE> {
        // SAFETY: GetStdHandle не имеет предусловий.
        let h = unsafe { GetStdHandle(which) };
        (!h.is_null() && h != INVALID_HANDLE_VALUE).then_some(h)
    }

    /// Прочитать ровно n байт (меньше — если поток закончился).
    pub fn read(n: usize) -> Vec<u8> {
        let mut out = vec![0u8; n];
        let Some(h) = handle(STD_INPUT_HANDLE) else { return Vec::new() };
        let mut have = 0usize;
        while have < n {
            let want = u32::try_from(n - have).unwrap_or(u32::MAX);
            let mut got = 0u32;
            // SAFETY: буфер out[have..] длиной не меньше want, got — валидный указатель.
            let ok = unsafe { ReadFile(h, out[have..].as_mut_ptr(), want, &mut got, std::ptr::null_mut()) };
            if ok == 0 || got == 0 {
                break;
            }
            have += got as usize;
        }
        out.truncate(have);
        out
    }

    pub fn write(data: &[u8]) {
        let Some(h) = handle(STD_OUTPUT_HANDLE) else { return };
        let mut done = 0usize;
        while done < data.len() {
            let want = u32::try_from(data.len() - done).unwrap_or(u32::MAX);
            let mut wrote = 0u32;
            // SAFETY: data[done..] валиден на want байт.
            let ok = unsafe { WriteFile(h, data[done..].as_ptr(), want, &mut wrote, std::ptr::null_mut()) };
            if ok == 0 || wrote == 0 {
                return;
            }
            done += wrote as usize;
        }
        // SAFETY: h — дескриптор stdout.
        unsafe { FlushFileBuffers(h) };
    }
}

/// Кадр native messaging: 4 байта длины (little-endian) + JSON в UTF-8.
pub fn frame(v: &Value) -> Vec<u8> {
    let data = serde_json::to_vec(v).unwrap_or_else(|_| b"{}".to_vec());
    let mut out = (data.len() as u32).to_le_bytes().to_vec();
    out.extend_from_slice(&data);
    out
}

/// Режим посредника: читать сообщения браузера из stdin, писать ответы в stdout. Возвращает код выхода.
/// Окна не создаёт. Должен ответить быстро (< 1 с), даже если запускает программу.
pub fn run_native_host() -> i32 {
    loop {
        let head = stdio::read(4);
        let Ok(head) = <[u8; 4]>::try_from(head.as_slice()) else { break };
        let n = u32::from_le_bytes(head) as usize;
        let body = stdio::read(n);
        let reply = match serde_json::from_slice::<Value>(&body) {
            Ok(msg) => host_message(&msg),
            Err(e) => json!({"ok": false, "error": e.to_string()}),
        };
        stdio::write(&frame(&reply));
    }
    0
}

// ---------------------------------------------------------------- сторона окна

/// Локальный сокет, через который процесс-посредник будит открытое окно.
/// Сервер работает, пока не вызван stop() (Drop его не останавливает).
pub struct IpcServer {
    port: u16,
    stopped: Arc<AtomicBool>,
}

impl IpcServer {
    /// Запустить сервер и записать ipc.json. on_wake вызывается из фонового потока при каждом «стуке».
    pub fn start(on_wake: Box<dyn Fn() + Send + Sync>) -> Option<IpcServer> {
        let listener = TcpListener::bind((Ipv4Addr::LOCALHOST, 0)).ok()?;
        let port = listener.local_addr().ok()?.port();
        let token: String = rand::random::<[u8; 16]>().iter().map(|b| format!("{b:02x}")).collect();
        std::fs::create_dir_all(paths::data_dir()).ok()?;
        let body = json!({"port": port, "token": token, "pid": std::process::id()});
        std::fs::write(ipc_file(), serde_json::to_string(&body).ok()?).ok()?;
        let stopped = Arc::new(AtomicBool::new(false));
        let flag = stopped.clone();
        std::thread::Builder::new()
            .name("topicture-ipc".into())
            .spawn(move || {
                for conn in listener.incoming() {
                    if flag.load(Ordering::SeqCst) {
                        return;
                    }
                    let Ok(mut c) = conn else { continue };
                    if read_token(&mut c).as_deref() == Some(token.as_str()) && c.write_all(b"ok").is_ok() {
                        drop(c);
                        on_wake();
                    }
                }
            })
            .ok()?;
        Some(IpcServer { port, stopped })
    }

    /// Порт, на котором слушает сервер.
    pub fn port(&self) -> u16 { self.port }

    /// Убрать ipc.json (если он наш) и остановить сервер. Вызывается при закрытии окна.
    pub fn stop(&self) {
        let f = ipc_file();
        if let Ok(text) = std::fs::read_to_string(&f) {
            let ours = serde_json::from_str::<Value>(&text)
                .ok()
                .and_then(|v| v.get("pid").and_then(Value::as_u64))
                .is_some_and(|pid| pid == u64::from(std::process::id()));
            if ours {
                let _ = std::fs::remove_file(&f);
            }
        }
        if !self.stopped.swap(true, Ordering::SeqCst) {
            // разбудить accept, чтобы поток увидел флаг и завершился
            let addr = SocketAddr::from((Ipv4Addr::LOCALHOST, self.port));
            let _ = TcpStream::connect_timeout(&addr, Duration::from_millis(500));
        }
    }
}

/// Прочитать токен (до 64 байт или до перевода строки), не дольше 2 с.
fn read_token(c: &mut TcpStream) -> Option<String> {
    c.set_read_timeout(Some(Duration::from_secs(2))).ok()?;
    let mut buf = [0u8; 64];
    let mut got = 0;
    while got < buf.len() {
        match c.read(&mut buf[got..]) {
            Ok(0) => break,
            Ok(n) => got += n,
            Err(_) => return None,
        }
        if buf[..got].contains(&b'\n') {
            break;
        }
    }
    Some(String::from_utf8_lossy(&buf[..got]).trim().to_string())
}

/// Забрать присланные ссылки из inbox (файлы удаляются сразу; старше часа — отбрасываются).
pub fn take_inbox() -> Vec<InboxLink> {
    take_inbox_from(&inbox_dir(), now())
}

/// То же для заданной папки и момента времени (для тестов).
pub fn take_inbox_from(dir: &Path, now: f64) -> Vec<InboxLink> {
    let Ok(rd) = std::fs::read_dir(dir) else { return Vec::new() };
    let mut files: Vec<PathBuf> = rd
        .filter_map(|e| e.ok().map(|e| e.path()))
        .filter(|p| p.extension().is_some_and(|x| x.eq_ignore_ascii_case("json")) && p.is_file())
        .collect();
    files.sort();
    let mut got = Vec::new();
    for f in files {
        let m = std::fs::read_to_string(&f)
            .ok()
            .and_then(|t| serde_json::from_str::<Value>(t.trim_start_matches('\u{feff}')).ok());
        let _ = std::fs::remove_file(&f); // cookies на диске не задерживаются
        let Some(Value::Object(m)) = m else { continue };
        let time = m.get("time").and_then(Value::as_f64).unwrap_or(0.0);
        if m.is_empty() || now - time > INBOX_MAX_AGE {
            continue;
        }
        let Some(url) = m.get("url").and_then(Value::as_str) else { continue };
        let cookies = m.get("cookies").filter(|c| truthy(Some(c))).map(netscape_cookies);
        got.push(InboxLink { url: url.to_string(), title: str_or_empty(m.get("title")), cookies });
    }
    got
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("topicture-bridge-unit-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn netscape_format() {
        let c = json!([
            {"domain": "youtube.com", "hostOnly": false, "path": "/", "secure": true, "httpOnly": true,
             "expirationDate": 1893456000.75, "name": "SID", "value": "abc"},
            {"domain": "www.youtube.com", "hostOnly": true, "path": "", "secure": false, "httpOnly": false,
             "expirationDate": 0, "name": "PREF", "value": "x=1"},
            {"domain": ".example.com", "name": "n", "value": "v"},
            {"name": "no-domain", "value": "v"},
            "garbage",
        ]);
        assert_eq!(
            netscape_cookies(&c),
            "# Netscape HTTP Cookie File\n\
             #HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1893456000\tSID\tabc\n\
             www.youtube.com\tFALSE\t/\tFALSE\t0\tPREF\tx=1\n\
             .example.com\tTRUE\t/\tFALSE\t0\tn\tv\n"
        );
        assert_eq!(netscape_cookies(&Value::Null), "# Netscape HTTP Cookie File\n");
        assert_eq!(netscape_cookies(&json!({"a": 1})), "# Netscape HTTP Cookie File\n");
    }

    #[test]
    fn site_keys() {
        use topicture_core::site_key;
        assert_eq!(site_key("https://www.youtube.com/watch?v=1"), "youtube.com");
        assert_eq!(site_key("https://m.youtube.com/watch?v=1"), "youtube.com");
        assert_eq!(site_key("https://youtu.be/abc"), "youtube.com");
        assert_eq!(site_key("https://VIMEO.com:443/1"), "vimeo.com");
    }

    #[test]
    fn messages() {
        let p = |v: Value| parse_message(v.as_object().unwrap());
        assert_eq!(p(json!({"type": "ping"})), Message::Ping);
        assert_eq!(p(json!({"type": "add", "url": "ftp://x"})), Message::Bad);
        assert_eq!(p(json!({"type": "add", "url": "HTTPS://x"})), Message::Bad); // как re.match без IGNORECASE
        assert_eq!(p(json!({"type": "add"})), Message::Bad);
        assert_eq!(p(json!({"type": "other", "url": "https://x"})), Message::Bad);
        assert_eq!(
            p(json!({"type": "add", "url": "https://x/y", "title": null, "cookies": null})),
            Message::Add { url: "https://x/y".into(), title: String::new(), cookies: json!([]) }
        );
        assert_eq!(
            p(json!({"type": "add", "url": "http://a", "title": "T", "cookies": [{"name": "a"}]})),
            Message::Add { url: "http://a".into(), title: "T".into(), cookies: json!([{"name": "a"}]) }
        );
        assert_eq!(host_message(&json!([1])), json!({"ok": false, "error": "bad message"}));
        assert_eq!(host_message(&json!({"type": "add", "url": "x"})), json!({"ok": false, "error": "bad url"}));
        assert_eq!(host_message(&json!({"type": "ping"}))["ok"], json!(true));
    }

    #[test]
    fn frames() {
        let f = frame(&json!({"ok": true}));
        assert_eq!(&f[..4], &11u32.to_le_bytes());
        assert_eq!(&f[4..], br#"{"ok":true}"#);
    }

    #[test]
    fn inbox_roundtrip_and_expiry() {
        let d = scratch("inbox");
        let c = json!([{"domain": "youtube.com", "hostOnly": false, "path": "/", "secure": true,
                        "httpOnly": false, "expirationDate": 5, "name": "a", "value": "b"}]);
        let fresh = write_inbox(&d, "https://youtu.be/x", "Тест", c).unwrap();
        assert!(!d.read_dir().unwrap().any(|e| e.unwrap().path().to_string_lossy().ends_with(".tmp")));
        let stored: Value = serde_json::from_str(&std::fs::read_to_string(&fresh).unwrap()).unwrap();
        assert_eq!(stored["title"], "Тест");
        assert!(stored["time"].is_f64());
        // старое (больше часа) и испорченное
        std::fs::write(d.join("0000000001.000000-1.json"), r#"{"url":"https://old","title":"","cookies":[],"time":1}"#)
            .unwrap();
        std::fs::write(d.join("0000000002.000000-1.json"), "not json").unwrap();
        std::fs::write(d.join("0000000003.000000-1.json"), r#"{"url":"https://notime"}"#).unwrap();
        std::fs::write(d.join("keep.json.tmp"), "{}").unwrap();
        let t = now();
        let got = take_inbox_from(&d, t);
        assert_eq!(got.len(), 1);
        assert_eq!(got[0].url, "https://youtu.be/x");
        assert_eq!(got[0].title, "Тест");
        assert_eq!(got[0].cookies.as_deref(), Some("# Netscape HTTP Cookie File\n.youtube.com\tTRUE\t/\tTRUE\t5\ta\tb\n"));
        // все .json удалены, .tmp не тронут
        let left: Vec<_> = d.read_dir().unwrap().map(|e| e.unwrap().file_name()).collect();
        assert_eq!(left, vec![std::ffi::OsString::from("keep.json.tmp")]);
        // граница: ровно час — ещё берём, больше — нет; без cookies — None
        write_inbox(&d, "https://a", "", json!([])).unwrap();
        assert!(take_inbox_from(&d, t + INBOX_MAX_AGE + 5.0).is_empty());
        write_inbox(&d, "https://a", "", json!([])).unwrap();
        let got = take_inbox_from(&d, t);
        assert_eq!(got.len(), 1);
        assert!(got[0].cookies.is_none());
        let _ = std::fs::remove_dir_all(&d);
    }

    #[test]
    fn paths_compare() {
        assert_eq!(norm_path(r"\\?\C:\A\B.exe"), norm_path("c:/a/b.EXE"));
        assert!(is_host_invocation(&["x".into(), "chrome-extension://abc/".into()]));
        assert!(!is_host_invocation(&["--register-extension".into()]));
    }
}
