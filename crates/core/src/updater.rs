//! Обновление собственного bin\yt-dlp.exe: раз в сутки при запуске и сразу,
//! если загрузка упала с ошибкой, похожей на «yt-dlp устарел».
use crate::{Event, LogLevel, Shared, paths, procs, settings};
use std::path::PathBuf;
use std::sync::{Condvar, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const UPDATE_EVERY: f64 = 20.0 * 3600.0; // раз в сутки (с запасом)

pub struct Updater {
    lock: Mutex<()>,
    idle: Mutex<bool>,
    cv: Condvar,
}

impl Default for Updater {
    fn default() -> Self { Updater { lock: Mutex::new(()), idle: Mutex::new(true), cv: Condvar::new() } }
}

pub fn own_ytdlp() -> PathBuf { paths::bin_dir().join("yt-dlp.exe") }

fn now() -> f64 { SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0) }

impl Updater {
    /// Обновляем только свой yt-dlp (не системный). TOPICTURE_NO_UPDATE=1 — выключено (для тестов).
    pub fn enabled(&self) -> bool {
        std::env::var_os("TOPICTURE_NO_UPDATE").is_none_or(|v| v.is_empty()) && own_ytdlp().is_file()
    }

    pub fn is_idle(&self) -> bool { *self.idle.lock().unwrap() }

    pub fn wait_idle(&self, timeout: Duration) {
        let g = self.idle.lock().unwrap();
        let _ = self.cv.wait_timeout_while(g, timeout, |idle| !*idle);
    }

    fn set_idle(&self, v: bool) {
        *self.idle.lock().unwrap() = v;
        self.cv.notify_all();
    }

    /// true, если yt-dlp действительно обновился.
    pub(crate) fn check(&self, sh: &Shared, force: bool) -> bool {
        if !self.enabled() {
            return false;
        }
        let _g = self.lock.lock().unwrap_or_else(|e| e.into_inner());
        let mut state = settings::load_state();
        let last = state.get("ytdlp_checked").and_then(|v| v.as_f64()).unwrap_or(0.0);
        if !force && now() - last < UPDATE_EVERY {
            return false;
        }
        self.set_idle(false);
        let lang = sh.lang();
        let log = |line: String, level| (sh.sink)(Event::Log { line, level });
        log(crate::i18n::tr(&lang, "py.updCheck", &[]), LogLevel::Normal);
        let exe = own_ytdlp().to_string_lossy().into_owned();
        let res = (|| -> std::io::Result<bool> {
            let r = procs::run_quiet(&[exe.clone(), "-U".into()], Duration::from_secs(240))?;
            let mut out = String::from_utf8_lossy(&r.stdout).into_owned();
            out.push_str(&String::from_utf8_lossy(&r.stderr));
            state.insert("ytdlp_checked".into(), serde_json::json!(now()));
            settings::save_state(&state);
            let updated = out.contains("Updated yt-dlp to");
            let failed = r.code != Some(0) && !updated;
            let v = procs::run_quiet(&[exe.clone(), "--version".into()], Duration::from_secs(30))?;
            let version = String::from_utf8_lossy(&v.stdout).trim().to_string();
            let msg = if updated {
                crate::i18n::tr(&lang, "py.updDone", &[("v", &version)])
            } else if failed {
                let last = out.trim().lines().rfind(|l| !l.trim().is_empty()).map(|l| format!(" — {l}"));
                crate::i18n::tr(&lang, "py.updFail", &[("e", last.as_deref().unwrap_or(""))])
            } else {
                crate::i18n::tr(&lang, "py.updLatest", &[("v", &version)])
            };
            // в Python — событие ytdlp; интерфейс показывал его как строку журнала
            log(msg, if failed { LogLevel::Error } else { LogLevel::Normal });
            Ok(updated)
        })();
        let updated = match res {
            Ok(u) => u,
            Err(e) => {
                log(crate::i18n::tr(&lang, "py.updErr", &[("e", &e.to_string())]), LogLevel::Error);
                false
            }
        };
        self.set_idle(true);
        updated
    }
}
