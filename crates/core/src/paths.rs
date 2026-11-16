//! Папки программы (как в ToPicture.pyw).
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// Папка установки: рядом с exe. При разработке (`cargo run`, exe в target\debug|release)
/// это корень репозитория, чтобы находились installer\payload\bin и extension.
pub fn install_dir() -> &'static Path {
    static D: OnceLock<PathBuf> = OnceLock::new();
    D.get_or_init(|| {
        if let Ok(p) = std::env::var("TOPICTURE_INSTALL") {
            return PathBuf::from(p);
        }
        let exe = std::env::current_exe().ok().and_then(|p| p.canonicalize().ok()).unwrap_or_default();
        let dir = exe.parent().map(Path::to_path_buf).unwrap_or_default();
        // target\debug\topicture.exe -> корень репозитория
        if dir.parent().and_then(|p| p.file_name()).is_some_and(|n| n == "target") {
            if let Some(root) = dir.parent().and_then(Path::parent) {
                return strip_unc(root);
            }
        }
        strip_unc(&dir)
    })
}

fn strip_unc(p: &Path) -> PathBuf {
    let s = p.to_string_lossy();
    PathBuf::from(s.strip_prefix(r"\\?\").unwrap_or(&s).to_string())
}

/// Свои yt-dlp.exe, ffmpeg.exe, ffprobe.exe. В установленной программе — {app}\bin,
/// при разработке — installer\payload\bin.
pub fn bin_dir() -> PathBuf {
    let d = install_dir().join("bin");
    if d.is_dir() { return d; }
    let dev = install_dir().join("installer").join("payload").join("bin");
    if dev.is_dir() { dev } else { d }
}

/// Портативный режим: рядом с exe лежит portable.txt.
pub fn portable() -> bool {
    install_dir().join("portable.txt").exists()
}

/// %APPDATA%\ToPicture, в портативном режиме — data рядом с exe; TOPICTURE_DATA — для тестов.
pub fn data_dir() -> PathBuf {
    if let Ok(p) = std::env::var("TOPICTURE_DATA") {
        if !p.is_empty() { return PathBuf::from(p); }
    }
    if portable() {
        return install_dir().join("data");
    }
    appdata().join("ToPicture")
}

pub fn appdata() -> PathBuf {
    std::env::var_os("APPDATA").map(PathBuf::from).unwrap_or_else(|| home())
}

pub fn home() -> PathBuf {
    std::env::var_os("USERPROFILE").map(PathBuf::from).unwrap_or_else(|| PathBuf::from("."))
}

pub fn config_file() -> PathBuf { data_dir().join("settings.json") }
pub fn state_file() -> PathBuf { data_dir().join("state.json") }
/// Настройки самой старой версии (YtdlCubase) — переносятся один раз.
pub fn old_config_file() -> Option<PathBuf> {
    if portable() { None } else { Some(appdata().join("YtdlCubase").join("settings.json")) }
}
/// Папка расширения для браузера ({app}\extension).
pub fn extension_dir() -> PathBuf { install_dir().join("extension") }

/// Путь к инструменту: сначала свой bin, потом PATH. None — не найден.
pub fn tool(name: &str) -> Option<PathBuf> {
    let exe = format!("{name}.exe");
    let own = bin_dir().join(&exe);
    if own.is_file() { return Some(own); }
    let path = std::env::var_os("PATH")?;
    std::env::split_paths(&path).map(|d| d.join(&exe)).find(|p| p.is_file())
}
