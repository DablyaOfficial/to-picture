//! To Picture — связь с расширением для браузера (native messaging), inbox и пробуждение окна.
//! Порт раздела «Bridge» из ToPicture.pyw. Формат файлов (native-host.json, inbox/*.json, ipc.json)
//! и разделы реестра — те же, что у Python-версии.
//!
//! КОНТРАКТ: публичные сигнатуры этого файла использует crate app. Только добавлять, не менять.

pub const HOST_NAME: &str = "com.topicture.app";
/// Постоянный ID: задан ключом в extension/manifest.json.
pub const EXT_ID: &str = "kefllcapjmcgojghjpaofcpoigeiocgi";

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

/// Аргументы, с которыми браузер запускает программу как native host (первый — chrome-extension://…).
pub fn is_host_invocation(args: &[String]) -> bool {
    args.iter().any(|a| a.starts_with("chrome-extension://"))
}

/// Режим посредника: читать сообщения браузера из stdin, писать ответы в stdout. Возвращает код выхода.
/// Окна не создаёт. Должен ответить быстро (< 1 с), даже если запускает программу.
pub fn run_native_host() -> i32 { todo!("bridge: run_native_host") }

/// Записать native-host.json и разделы реестра (HKCU) для всех браузеров на Chromium.
pub fn register_extension() -> std::io::Result<()> { todo!("bridge: register_extension") }
pub fn unregister_extension() { todo!("bridge: unregister_extension") }
pub fn extension_registered() -> bool { todo!("bridge: extension_registered") }
pub fn ext_status() -> ExtStatus { todo!("bridge: ext_status") }

/// Локальный сокет, через который процесс-посредник будит открытое окно.
pub struct IpcServer { _private: () }

impl IpcServer {
    /// Запустить сервер и записать ipc.json. on_wake вызывается из фонового потока при каждом «стуке».
    pub fn start(on_wake: Box<dyn Fn() + Send + Sync>) -> Option<IpcServer> { let _ = on_wake; todo!("bridge: IpcServer::start") }
    /// Убрать ipc.json (если он наш) и остановить сервер. Вызывается при закрытии окна.
    pub fn stop(&self) { todo!("bridge: IpcServer::stop") }
}

/// Забрать присланные ссылки из inbox (файлы удаляются сразу; старше часа — отбрасываются).
pub fn take_inbox() -> Vec<InboxLink> { todo!("bridge: take_inbox") }

/// Показать один раз панель подключения расширения: связь есть, им не пользовались, панель не показывали.
/// Сам ставит отметку ext_help_shown.
pub fn should_show_help_once() -> bool { todo!("bridge: should_show_help_once") }
