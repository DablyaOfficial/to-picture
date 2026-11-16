//! Дочерние процессы: запуск без окна, дерево процессов (Toolhelp32), пауза (NtSuspendProcess),
//! остановка всего дерева. Трогаем только процессы, запущенные нами, и их потомков.
use crate::paths;
use std::io::Read;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};
use windows_sys::Win32::Foundation::{CloseHandle, FILETIME, HANDLE, INVALID_HANDLE_VALUE, WAIT_TIMEOUT};
use windows_sys::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, PROCESSENTRY32W, Process32FirstW, Process32NextW, TH32CS_SNAPPROCESS,
};
use windows_sys::Win32::System::Threading::{
    BELOW_NORMAL_PRIORITY_CLASS, CREATE_NO_WINDOW, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
    PROCESS_SUSPEND_RESUME, PROCESS_TERMINATE, TerminateProcess, WaitForSingleObject,
};

#[link(name = "ntdll", kind = "raw-dylib")]
unsafe extern "system" {
    fn NtSuspendProcess(h: HANDLE) -> i32;
    fn NtResumeProcess(h: HANDLE) -> i32;
}

pub const NO_WINDOW: u32 = CREATE_NO_WINDOW;
pub const BELOW_NORMAL: u32 = BELOW_NORMAL_PRIORITY_CLASS;

/// "yt-dlp" / "ffmpeg" / "ffprobe" -> свой bin, потом PATH; иначе имя как есть (запуск выдаст ошибку).
pub fn resolve(name: &str) -> PathBuf {
    if name.contains(['\\', '/']) {
        return PathBuf::from(name);
    }
    paths::tool(name).unwrap_or_else(|| PathBuf::from(name))
}

/// Команда как в Python: без окна, UTF-8 для yt-dlp, stdin пустой.
pub fn command(args: &[String], flags: u32) -> Command {
    let mut c = Command::new(resolve(&args[0]));
    c.args(&args[1..])
        .stdin(Stdio::null())
        .creation_flags(flags)
        .env("PYTHONIOENCODING", "utf-8")
        .env("PYTHONUTF8", "1");
    c
}

pub struct Output {
    pub code: Option<i32>,
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
}

/// subprocess.run(capture_output=True, timeout=…). По таймауту дерево процесса останавливается.
pub fn run_quiet(args: &[String], timeout: Duration) -> std::io::Result<Output> {
    let mut child = command(args, NO_WINDOW).stdout(Stdio::piped()).stderr(Stdio::piped()).spawn()?;
    let mut so = child.stdout.take().unwrap();
    let mut se = child.stderr.take().unwrap();
    let t1 = std::thread::spawn(move || {
        let mut v = vec![];
        let _ = so.read_to_end(&mut v);
        v
    });
    let t2 = std::thread::spawn(move || {
        let mut v = vec![];
        let _ = se.read_to_end(&mut v);
        v
    });
    let t0 = Instant::now();
    let status = loop {
        if let Some(st) = child.try_wait()? {
            break st;
        }
        if t0.elapsed() > timeout {
            kill_tree(child.id());
            let _ = child.kill();
            let _ = child.wait();
            return Err(std::io::Error::new(std::io::ErrorKind::TimedOut, "timed out"));
        }
        std::thread::sleep(Duration::from_millis(30));
    };
    Ok(Output { code: status.code(), stdout: t1.join().unwrap_or_default(), stderr: t2.join().unwrap_or_default() })
}

fn snapshot() -> Vec<(u32, u32)> {
    let mut out = vec![];
    unsafe {
        let snap = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0);
        if snap == INVALID_HANDLE_VALUE || snap.is_null() {
            return out;
        }
        let mut pe: PROCESSENTRY32W = std::mem::zeroed();
        pe.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut ok = Process32FirstW(snap, &mut pe);
        while ok != 0 {
            out.push((pe.th32ProcessID, pe.th32ParentProcessID));
            ok = Process32NextW(snap, &mut pe);
        }
        CloseHandle(snap);
    }
    out
}

fn creation_time(pid: u32) -> Option<u64> {
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return None;
        }
        let z = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let (mut c, mut e, mut k, mut u) = (z, z, z, z);
        let ok = GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u);
        CloseHandle(h);
        (ok != 0).then_some(((c.dwHighDateTime as u64) << 32) | c.dwLowDateTime as u64)
    }
}

/// pid и все его потомки (yt-dlp.exe запускает себя второй раз, а потом ffmpeg).
/// Потомок учитывается, только если создан не раньше родителя (защита от повторно выданных PID).
pub fn process_tree(root: u32) -> Vec<u32> {
    let all = snapshot();
    let root_t = creation_time(root);
    let mut out = vec![root];
    let mut todo = vec![root];
    while let Some(p) = todo.pop() {
        let bound = creation_time(p).or(root_t);
        for &(c, pp) in &all {
            if pp != p || c == 0 || c == p || out.contains(&c) {
                continue;
            }
            if let (Some(b), Some(ct)) = (bound, creation_time(c)) {
                if ct < b {
                    continue;
                }
            }
            out.push(c);
            todo.push(c);
        }
    }
    out
}

/// Пауза: процессы замораживаются целиком (загрузка и перекодирование продолжаются с того же места).
pub fn suspend_tree(pid: u32, suspend: bool) {
    for p in process_tree(pid) {
        unsafe {
            let h = OpenProcess(PROCESS_SUSPEND_RESUME, 0, p);
            if !h.is_null() {
                if suspend { NtSuspendProcess(h) } else { NtResumeProcess(h) };
                CloseHandle(h);
            }
        }
    }
}

/// Остановить процесс и всех потомков (как taskkill /F /T). Сначала дерево замораживается,
/// чтобы никто не успел запустить новый процесс.
pub fn kill_tree(pid: u32) {
    for _ in 0..2 {
        let tree = process_tree(pid);
        for &p in &tree {
            unsafe {
                let h = OpenProcess(PROCESS_SUSPEND_RESUME, 0, p);
                if !h.is_null() {
                    NtSuspendProcess(h);
                    CloseHandle(h);
                }
            }
        }
        for &p in &tree {
            unsafe {
                let h = OpenProcess(PROCESS_TERMINATE, 0, p);
                if !h.is_null() {
                    TerminateProcess(h, 1);
                    CloseHandle(h);
                }
            }
        }
    }
}

/// Процесс ещё работает (handle — от std::process::Child, пока он жив).
pub fn alive(handle: usize) -> bool {
    unsafe { WaitForSingleObject(handle as HANDLE, 0) == WAIT_TIMEOUT }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::io::AsRawHandle;

    #[test]
    fn tree_suspend_kill() {
        // cmd -> ping (долгий): дерево из двух процессов, оба наши
        let args: Vec<String> = ["cmd", "/c", "ping -n 30 127.0.0.1 >nul"].iter().map(|s| s.to_string()).collect();
        let mut c = command(&args, NO_WINDOW).spawn().unwrap();
        let t0 = Instant::now();
        let mut tree = vec![];
        while t0.elapsed() < Duration::from_secs(5) {
            tree = process_tree(c.id());
            if tree.len() >= 2 { break; }
            std::thread::sleep(Duration::from_millis(50));
        }
        assert!(tree.len() >= 2, "{tree:?}");
        suspend_tree(c.id(), true);
        suspend_tree(c.id(), false);
        assert!(alive(c.as_raw_handle() as usize));
        kill_tree(c.id());
        let st = c.wait().unwrap();
        assert!(!st.success());
        std::thread::sleep(Duration::from_millis(200));
        let left: Vec<u32> = snapshot().iter().map(|x| x.0).filter(|p| tree.contains(p)).collect();
        assert!(left.is_empty(), "left {left:?}");
    }

    #[test]
    fn quiet_timeout() {
        let args: Vec<String> = ["cmd", "/c", "echo hi"].iter().map(|s| s.to_string()).collect();
        let o = run_quiet(&args, Duration::from_secs(10)).unwrap();
        assert_eq!(o.code, Some(0));
        assert_eq!(String::from_utf8_lossy(&o.stdout).trim(), "hi");
        let args: Vec<String> = ["cmd", "/c", "ping -n 20 127.0.0.1 >nul"].iter().map(|s| s.to_string()).collect();
        let t = Instant::now();
        assert!(run_quiet(&args, Duration::from_millis(500)).is_err());
        assert!(t.elapsed() < Duration::from_secs(5));
    }
}
