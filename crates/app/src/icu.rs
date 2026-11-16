//! Названия языков, сокращённые числа и даты на языке интерфейса — через icu.dll из Windows 10/11
//! (замена Intl.DisplayNames / Intl.NumberFormat / Intl.DateTimeFormat из index.html).
//! Если icu.dll нет — простые запасные варианты.
#![allow(non_camel_case_types)]
use std::cell::RefCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CString};
use std::sync::OnceLock;

type UErr = i32;
type FnUldnOpen = unsafe extern "C" fn(*const c_char, i32, *mut UErr) -> *mut c_void;
type FnUldnLang = unsafe extern "C" fn(*const c_void, *const c_char, *mut u16, i32, *mut UErr) -> i32;
type FnNumfOpen = unsafe extern "C" fn(*const u16, i32, *const c_char, *mut UErr) -> *mut c_void;
type FnNumfOpenResult = unsafe extern "C" fn(*mut UErr) -> *mut c_void;
type FnNumfFormatDouble = unsafe extern "C" fn(*const c_void, f64, *mut c_void, *mut UErr);
type FnNumfResultToString = unsafe extern "C" fn(*const c_void, *mut u16, i32, *mut UErr) -> i32;
type FnDtpgOpen = unsafe extern "C" fn(*const c_char, *mut UErr) -> *mut c_void;
type FnDtpgBest = unsafe extern "C" fn(*mut c_void, *const u16, i32, *mut u16, i32, *mut UErr) -> i32;
type FnDatOpen = unsafe extern "C" fn(i32, i32, *const c_char, *const u16, i32, *const u16, i32, *mut UErr) -> *mut c_void;
type FnDatFormat = unsafe extern "C" fn(*const c_void, f64, *mut u16, i32, *mut c_void, *mut UErr) -> i32;

struct Icu {
    uldn_open: FnUldnOpen,
    uldn_lang: FnUldnLang,
    numf_open: FnNumfOpen,
    numf_open_result: FnNumfOpenResult,
    numf_format_double: FnNumfFormatDouble,
    numf_result_to_string: FnNumfResultToString,
    dtpg_open: FnDtpgOpen,
    dtpg_best: FnDtpgBest,
    dat_open: FnDatOpen,
    dat_format: FnDatFormat,
}

// указатели на функции из системной библиотеки — потокобезопасны
unsafe impl Send for Icu {}
unsafe impl Sync for Icu {}

fn icu() -> Option<&'static Icu> {
    static I: OnceLock<Option<Icu>> = OnceLock::new();
    I.get_or_init(load).as_ref()
}

#[cfg(windows)]
fn load() -> Option<Icu> {
    use windows_sys::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryExW, LOAD_LIBRARY_SEARCH_SYSTEM32};
    let name: Vec<u16> = "icu.dll\0".encode_utf16().collect();
    let h = unsafe { LoadLibraryExW(name.as_ptr(), std::ptr::null_mut(), LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if h.is_null() {
        return None;
    }
    macro_rules! f {
        ($n:literal, $t:ty) => {{
            let p = unsafe { GetProcAddress(h, concat!($n, "\0").as_ptr()) }?;
            unsafe { std::mem::transmute::<unsafe extern "system" fn() -> isize, $t>(p) }
        }};
    }
    Some(Icu {
        uldn_open: f!("uldn_open", FnUldnOpen),
        uldn_lang: f!("uldn_languageDisplayName", FnUldnLang),
        numf_open: f!("unumf_openForSkeletonAndLocale", FnNumfOpen),
        numf_open_result: f!("unumf_openResult", FnNumfOpenResult),
        numf_format_double: f!("unumf_formatDouble", FnNumfFormatDouble),
        numf_result_to_string: f!("unumf_resultToString", FnNumfResultToString),
        dtpg_open: f!("udatpg_open", FnDtpgOpen),
        dtpg_best: f!("udatpg_getBestPattern", FnDtpgBest),
        dat_open: f!("udat_open", FnDatOpen),
        dat_format: f!("udat_format", FnDatFormat),
    })
}

#[cfg(not(windows))]
fn load() -> Option<Icu> { None }

/// "pt-BR" -> "pt_BR"
fn loc(lang: &str) -> CString {
    CString::new(lang.replace('-', "_")).unwrap_or_default()
}

fn u16s(s: &str) -> Vec<u16> { s.encode_utf16().collect() }

// Объекты ICU кешируются на поток интерфейса и живут до выхода.
thread_local! {
    static CACHE: RefCell<HashMap<String, usize>> = RefCell::new(HashMap::new());
}

fn cached(key: String, make: impl FnOnce() -> *mut c_void) -> Option<*mut c_void> {
    CACHE.with(|c| {
        let mut c = c.borrow_mut();
        if let Some(p) = c.get(&key) {
            return (*p != 0).then_some(*p as *mut c_void);
        }
        let p = make();
        c.insert(key, p as usize);
        (!p.is_null()).then_some(p)
    })
}

/// Название языка `code` на языке `ui` (как Intl.DisplayNames).
pub fn language_name(ui: &str, code: &str) -> Option<String> {
    let i = icu()?;
    let h = cached(format!("ldn:{ui}"), || {
        let mut e = 0;
        let p = unsafe { (i.uldn_open)(loc(ui).as_ptr(), 0, &mut e) };
        if e > 0 { std::ptr::null_mut() } else { p }
    })?;
    let mut buf = [0u16; 128];
    let mut e = 0;
    let c = loc(code);
    let n = unsafe { (i.uldn_lang)(h, c.as_ptr(), buf.as_mut_ptr(), buf.len() as i32, &mut e) };
    if e > 0 || n <= 0 {
        return None;
    }
    let s = String::from_utf16_lossy(&buf[..n as usize]);
    // ICU возвращает сам код, если названия не знает
    (s != code && s != c.to_string_lossy()).then_some(s)
}

/// Число по «скелету» ICU: "compact-short .#", ".0", "precision-integer" …
pub fn number(lang: &str, skeleton: &str, v: f64) -> Option<String> {
    let i = icu()?;
    let h = cached(format!("num:{lang}:{skeleton}"), || {
        let sk = u16s(skeleton);
        let mut e = 0;
        let p = unsafe { (i.numf_open)(sk.as_ptr(), sk.len() as i32, loc(lang).as_ptr(), &mut e) };
        if e > 0 { std::ptr::null_mut() } else { p }
    })?;
    let r = cached("numres".into(), || {
        let mut e = 0;
        unsafe { (i.numf_open_result)(&mut e) }
    })?;
    let mut e = 0;
    unsafe { (i.numf_format_double)(h, v, r, &mut e) };
    if e > 0 {
        return None;
    }
    let mut buf = [0u16; 64];
    let n = unsafe { (i.numf_result_to_string)(r, buf.as_mut_ptr(), buf.len() as i32, &mut e) };
    (e <= 0 && n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
}

/// Дата/время по скелету шаблона ("yMMMd", "yMMMdjmm"); ms — миллисекунды с 1970 года.
pub fn date(lang: &str, skeleton: &str, ms: f64, utc: bool) -> Option<String> {
    let i = icu()?;
    let h = cached(format!("dat:{lang}:{skeleton}:{utc}"), || {
        let mut e = 0;
        let g = unsafe { (i.dtpg_open)(loc(lang).as_ptr(), &mut e) };
        if e > 0 || g.is_null() {
            return std::ptr::null_mut();
        }
        let sk = u16s(skeleton);
        let mut pat = [0u16; 128];
        let n = unsafe { (i.dtpg_best)(g, sk.as_ptr(), sk.len() as i32, pat.as_mut_ptr(), pat.len() as i32, &mut e) };
        if e > 0 || n <= 0 {
            return std::ptr::null_mut();
        }
        let tz = u16s("UTC");
        let (tzp, tzl) = if utc { (tz.as_ptr(), tz.len() as i32) } else { (std::ptr::null(), 0) };
        // UDAT_PATTERN = -2
        let p = unsafe { (i.dat_open)(-2, -2, loc(lang).as_ptr(), tzp, tzl, pat.as_ptr(), n, &mut e) };
        if e > 0 { std::ptr::null_mut() } else { p }
    })?;
    let mut buf = [0u16; 128];
    let mut e = 0;
    let n = unsafe { (i.dat_format)(h, ms, buf.as_mut_ptr(), buf.len() as i32, std::ptr::null_mut(), &mut e) };
    (e <= 0 && n > 0).then(|| String::from_utf16_lossy(&buf[..n as usize]))
}
