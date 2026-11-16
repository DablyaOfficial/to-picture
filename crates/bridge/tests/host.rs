//! Посредник через настоящие каналы stdin/stdout + пробуждение окна + inbox; регистрация в реестре.
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::{Duration, Instant};

use serde_json::{Value, json};
use topicture_bridge as bridge;

const DATA: &str = r"C:\Users\IUDANOV\AppData\Local\Temp\claude\8eeae023258e12cf\fc74b9c7-7e8e-47a8-a548-820b4c9d19bc\scratchpad\bridge";

fn setup() -> PathBuf {
    static ONCE: std::sync::Once = std::sync::Once::new();
    ONCE.call_once(|| {
        let _ = std::fs::remove_dir_all(DATA);
        std::fs::create_dir_all(DATA).unwrap();
        // SAFETY: один раз, до того как тесты начнут читать окружение.
        unsafe {
            std::env::set_var("TOPICTURE_DATA", DATA);
            // корень репозитория: там есть папка extension
            std::env::set_var("TOPICTURE_INSTALL", concat!(env!("CARGO_MANIFEST_DIR"), r"\..\.."));
        }
    });
    PathBuf::from(DATA)
}

/// Тесты делят state.json и папку данных — по очереди.
static LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn host_exe() -> PathBuf {
    // target\…\debug\deps\host-xxxx.exe -> target\…\debug\examples\host.exe
    // (или …\debug\build\topicture-bridge\<hash>\out\host-xxxx.exe при отдельном build-dir)
    let me = std::env::current_exe().unwrap();
    me.ancestors()
        .map(|a| a.join("examples").join("host.exe"))
        .find(|p| p.is_file())
        .unwrap_or_default()
}

fn send_raw(w: &mut impl Write, data: &[u8]) {
    w.write_all(&(data.len() as u32).to_le_bytes()).unwrap();
    w.write_all(data).unwrap();
    w.flush().unwrap();
}

fn send(w: &mut impl Write, v: &Value) {
    send_raw(w, &serde_json::to_vec(v).unwrap());
}

fn recv(r: &mut impl Read) -> Value {
    let mut head = [0u8; 4];
    r.read_exact(&mut head).unwrap();
    let mut body = vec![0u8; u32::from_le_bytes(head) as usize];
    r.read_exact(&mut body).unwrap();
    serde_json::from_slice(&body).unwrap()
}

fn files_in(dir: &Path) -> Vec<PathBuf> {
    std::fs::read_dir(dir).map(|rd| rd.map(|e| e.unwrap().path()).collect()).unwrap_or_default()
}

#[test]
fn native_host_wakes_window_and_fills_inbox() {
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = setup();
    let inbox = data.join("inbox");
    let ipc_path = data.join("ipc.json");
    let exe = host_exe();
    assert!(exe.is_file(), "нет {exe:?}: cargo test собирает examples");

    let (tx, rx) = mpsc::channel::<()>();
    let tx = std::sync::Mutex::new(tx);
    let server = bridge::IpcServer::start(Box::new(move || {
        let _ = tx.lock().unwrap().send(());
    }))
    .expect("ipc");
    let ipc: Value = serde_json::from_str(&std::fs::read_to_string(&ipc_path).unwrap()).unwrap();
    assert_eq!(ipc["port"].as_u64(), Some(u64::from(server.port())));
    let token = ipc["token"].as_str().unwrap();
    assert_eq!(token.len(), 32);
    assert!(token.chars().all(|c| c.is_ascii_hexdigit()));
    assert_eq!(ipc["pid"].as_u64(), Some(u64::from(std::process::id())));

    // чужой токен не будит
    {
        let mut s = std::net::TcpStream::connect(("127.0.0.1", server.port())).unwrap();
        s.write_all(b"wrong\n").unwrap();
        let mut buf = Vec::new();
        s.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        let _ = s.read_to_end(&mut buf);
        assert!(buf.is_empty());
        assert!(rx.recv_timeout(Duration::from_millis(200)).is_err());
    }

    let mut child = Command::new(&exe)
        .arg(format!("chrome-extension://{}/", bridge::EXT_ID))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let mut w = child.stdin.take().unwrap();
    let mut r = child.stdout.take().unwrap();

    send(&mut w, &json!({"type": "ping"}));
    let pong = recv(&mut r);
    assert_eq!(pong["ok"], json!(true));
    assert_eq!(pong["version"], json!(topicture_core::APP_VERSION));

    let cookies = json!([
        {"domain": ".youtube.com", "hostOnly": false, "path": "/", "secure": true, "httpOnly": true,
         "expirationDate": 1893456000.5, "name": "SID", "value": "secret"},
        {"domain": "www.youtube.com", "hostOnly": true, "path": "/", "secure": false, "httpOnly": false,
         "expirationDate": 0, "name": "PREF", "value": "hl=ru"},
    ]);
    let t0 = Instant::now();
    send(&mut w, &json!({"type": "add", "url": "https://www.youtube.com/watch?v=abc", "title": "Видео", "cookies": cookies}));
    let reply = recv(&mut r);
    assert!(t0.elapsed() < Duration::from_secs(1), "ответ дольше секунды: {:?}", t0.elapsed());
    assert_eq!(reply, json!({"ok": true, "launched": false}));
    rx.recv_timeout(Duration::from_secs(3)).expect("on_wake не вызван");

    let files = files_in(&inbox);
    assert_eq!(files.len(), 1, "{files:?}");
    let name = files[0].file_name().unwrap().to_string_lossy().to_string();
    assert!(name.ends_with(&format!("-{}.json", child.id())), "{name}");
    let state: Value = serde_json::from_str(&std::fs::read_to_string(data.join("state.json")).unwrap()).unwrap();
    assert!(state["ext_seen"].as_f64().unwrap() > 1.7e9);

    let links = bridge::take_inbox();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "https://www.youtube.com/watch?v=abc");
    assert_eq!(links[0].title, "Видео");
    assert_eq!(
        links[0].cookies.as_deref(),
        Some(
            "# Netscape HTTP Cookie File\n#HttpOnly_.youtube.com\tTRUE\t/\tTRUE\t1893456000\tSID\tsecret\n\
             www.youtube.com\tFALSE\t/\tFALSE\t0\tPREF\thl=ru\n"
        )
    );
    assert_eq!(topicture_core::site_key(&links[0].url), "youtube.com");
    assert!(files_in(&inbox).is_empty(), "inbox не очищен");
    assert!(bridge::take_inbox().is_empty());

    // плохие сообщения
    send(&mut w, &json!({"type": "add", "url": "javascript:alert(1)"}));
    assert_eq!(recv(&mut r), json!({"ok": false, "error": "bad url"}));
    send(&mut w, &json!("str"));
    assert_eq!(recv(&mut r), json!({"ok": false, "error": "bad message"}));
    send_raw(&mut w, b"{broken");
    let e = recv(&mut r);
    assert_eq!(e["ok"], json!(false));
    assert!(e["error"].as_str().is_some_and(|s| !s.is_empty()));
    assert!(files_in(&inbox).is_empty());

    // stop(): ipc.json чужого процесса не трогаем, свой — удаляем
    let mut foreign = ipc.clone();
    foreign["pid"] = json!(1);
    std::fs::write(&ipc_path, foreign.to_string()).unwrap();
    server.stop();
    assert!(ipc_path.exists(), "чужой ipc.json удалён");
    std::fs::remove_file(&ipc_path).unwrap();
    let server2 = bridge::IpcServer::start(Box::new(|| {})).expect("ipc 2");
    server2.stop();
    assert!(!ipc_path.exists(), "свой ipc.json не удалён");

    // окна нет: посредник запускает программу (здесь — тот же host.exe без аргументов, он сразу выходит)
    let t0 = Instant::now();
    send(&mut w, &json!({"type": "add", "url": "https://vimeo.com/1", "title": "", "cookies": []}));
    let reply = recv(&mut r);
    assert!(t0.elapsed() < Duration::from_secs(1), "ответ дольше секунды: {:?}", t0.elapsed());
    assert_eq!(reply, json!({"ok": true, "launched": true}));
    assert!(rx.recv_timeout(Duration::from_millis(300)).is_err(), "остановленный сервер не должен будить");
    let links = bridge::take_inbox();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].url, "https://vimeo.com/1");
    assert!(links[0].cookies.is_none());

    drop(w); // конец stdin -> посредник выходит с кодом 0
    assert_eq!(child.wait().unwrap().code(), Some(0));
}

// ---------------------------------------------------------------- реестр: сохранить и вернуть как было
mod reg {
    use winreg::RegKey;
    use winreg::enums::{HKEY_CURRENT_USER, KEY_READ};

    pub struct Snapshot {
        /// (путь, существовал ли) для каждого предка и самого раздела, от корня вглубь
        chain: Vec<(String, bool)>,
        values: Option<Vec<(String, winreg::RegValue<'static>)>>,
        full: String,
    }

    pub fn snapshot(full: &str) -> Snapshot {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let parts: Vec<&str> = full.split('\\').collect();
        let chain = (1..=parts.len())
            .map(|i| {
                let p = parts[..i].join("\\");
                let exists = hkcu.open_subkey_with_flags(&p, KEY_READ).is_ok();
                (p, exists)
            })
            .collect();
        let values = hkcu.open_subkey_with_flags(full, KEY_READ).ok().map(|k| {
            k.enum_values()
                .filter_map(Result::ok)
                .map(|(n, v)| (n, winreg::RegValue { bytes: v.bytes.into_owned().into(), vtype: v.vtype }))
                .collect()
        });
        Snapshot { chain, values, full: full.to_string() }
    }

    pub fn restore(s: &Snapshot) {
        let hkcu = RegKey::predef(HKEY_CURRENT_USER);
        let _ = hkcu.delete_subkey_all(&s.full);
        if let Some(values) = &s.values {
            let (k, _) = hkcu.create_subkey(&s.full).expect("восстановить раздел");
            for (n, v) in values {
                k.set_raw_value(n, v).expect("восстановить значение");
            }
        } else {
            // убрать созданных нами предков (только пустых), от глубоких к корню
            for (p, existed) in s.chain.iter().rev() {
                if !existed {
                    let _ = hkcu.delete_subkey(p);
                }
            }
        }
    }

    pub fn default_value(full: &str) -> Option<String> {
        RegKey::predef(HKEY_CURRENT_USER).open_subkey_with_flags(full, KEY_READ).ok()?.get_value("").ok()
    }
}

struct RegGuard(Vec<reg::Snapshot>);
impl Drop for RegGuard {
    fn drop(&mut self) {
        for s in &self.0 {
            reg::restore(s);
        }
    }
}

#[test]
fn register_and_unregister_restores_registry() {
    let _lock = LOCK.lock().unwrap_or_else(|e| e.into_inner());
    let data = setup();
    let keys: Vec<String> = bridge::HOST_KEYS.iter().map(|k| format!(r"{k}\{}", bridge::HOST_NAME)).collect();
    let before: Vec<Option<String>> = keys.iter().map(|k| reg::default_value(k)).collect();
    {
        let _guard = RegGuard(keys.iter().map(|k| reg::snapshot(k)).collect());

        bridge::register_extension().unwrap();
        let manifest = data.join("native-host.json");
        for k in &keys {
            assert_eq!(reg::default_value(k).as_deref(), Some(manifest.to_string_lossy().as_ref()));
        }
        let m: Value = serde_json::from_str(&std::fs::read_to_string(&manifest).unwrap()).unwrap();
        assert_eq!(m["name"], "com.topicture.app");
        assert_eq!(m["description"], "To Picture");
        assert_eq!(m["type"], "stdio");
        assert_eq!(m["allowed_origins"], json!([format!("chrome-extension://{}/", bridge::EXT_ID)]));
        assert_eq!(
            m["path"].as_str().unwrap().to_lowercase(),
            std::env::current_exe().unwrap().to_string_lossy().to_lowercase()
        );
        assert!(bridge::extension_registered());
        let st = bridge::ext_status();
        assert!(st.registered);
        assert_eq!(st.id, bridge::EXT_ID);
        assert!(st.has_folder, "{}", st.folder);

        // панель подсказки — один раз и только если расширением не пользовались
        let _ = std::fs::remove_file(data.join("state.json"));
        assert!(!bridge::ext_status().seen);
        assert!(bridge::should_show_help_once());
        assert!(!bridge::should_show_help_once());
        let state: Value = serde_json::from_str(&std::fs::read_to_string(data.join("state.json")).unwrap()).unwrap();
        assert_eq!(state["ext_help_shown"], json!(true));
        std::fs::write(data.join("state.json"), r#"{"ext_seen": 1700000000.5}"#).unwrap();
        assert!(bridge::ext_status().seen);
        assert!(!bridge::should_show_help_once());

        // путь к другому exe — уже не «наша» связь
        let mut other = m.clone();
        other["path"] = json!(r"C:\Elsewhere\To Picture.exe");
        std::fs::write(&manifest, serde_json::to_string_pretty(&other).unwrap()).unwrap();
        assert!(!bridge::extension_registered());

        bridge::unregister_extension();
        for k in &keys {
            assert!(reg::default_value(k).is_none(), "{k} остался");
        }
        assert!(!manifest.exists());
        assert!(!bridge::extension_registered());
    }
    let after: Vec<Option<String>> = keys.iter().map(|k| reg::default_value(k)).collect();
    assert_eq!(before, after, "реестр не восстановлен");
}
