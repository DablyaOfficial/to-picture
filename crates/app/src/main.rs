// To Picture — окно на Slint (без WebView). Загрузка видео: yt-dlp + ffmpeg, движок — crate topicture-core.
#![windows_subsystem = "windows"]

mod app;
mod backend;
mod demo;
mod fmt;
mod icu;
mod win;

use std::sync::Arc;
use topicture_bridge as bridge;

slint::include_modules!();

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    // Браузер запускает программу посредником native messaging: только stdin/stdout, без окна.
    // Подсистема windows здесь не мешает: браузер передаёт процессу каналы, и стандартные дескрипторы
    // наследуются (GetStdHandle возвращает их), консоль для этого не нужна.
    if bridge::is_host_invocation(&args) {
        std::process::exit(bridge::run_native_host());
    }
    // Установщик: связь с расширением в реестре.
    if args.iter().any(|a| a == "--register-extension") {
        std::process::exit(if bridge::register_extension().is_ok() { 0 } else { 1 });
    }
    if args.iter().any(|a| a == "--unregister-extension") {
        bridge::unregister_extension();
        std::process::exit(0);
    }

    win::set_app_id();
    // Skia на процессоре: окно 820×760 рисуется быстро, а памяти в простое ~15 МБ против ~240 МБ
    // у отрисовки через видеокарту (драйверы D3D/Vulkan резервируют много). SLINT_BACKEND может переопределить.
    if std::env::var_os("SLINT_BACKEND").is_none() {
        let _ = slint::BackendSelector::new().backend_name("winit".into()).renderer_name("skia-software".into()).select();
    }
    let demo = std::env::var("TOPICTURE_DEMO").is_ok_and(|v| v == "1");
    let sink: topicture_core::EventSink = Arc::new(|ev| app::post(move |a| a.on_event(ev)));
    let be: Arc<dyn backend::Backend> = if demo { Arc::new(demo::Demo::new(sink)) } else { Arc::new(backend::Real::new(sink)) };

    let ui = AppWindow::new().expect("window");
    app::App::install(&ui, be);
    ui.window().on_close_requested(|| {
        app::with_app(|a| a.closing());
        // таймеры (например, слежение за темой Windows) иначе держат цикл событий, и процесс остаётся без окна
        let _ = slint::quit_event_loop();
        slint::CloseRequestResponse::HideWindow
    });
    ui.show().expect("show");
    app::with_app(|a| a.after_show());
    // сценарий — для демо и для проверок настоящего движка (только с отдельной папкой данных TOPICTURE_DATA)
    let test_env = demo || std::env::var_os("TOPICTURE_DATA").is_some();
    if let Some(script) = std::env::var("TOPICTURE_DEMO_AUTO").ok().filter(|_| test_env) {
        demo_auto(&script);
    }
    slint::run_event_loop().expect("event loop");
    // фоновые потоки (превью, проверка yt-dlp) не должны держать процесс после закрытия окна;
    // очередь уже отменена в closing(), её процессы завершены
    std::process::exit(0);
}

/// Демо: действия по расписанию для снимков окна (TOPICTURE_DEMO_AUTO="300:paste=URL;1500:settings;…").
fn demo_auto(script: &str) {
    for step in script.split(';').filter(|s| !s.is_empty()) {
        let Some((ms, act)) = step.split_once(':') else { continue };
        let ms: u64 = ms.trim().parse().unwrap_or(0);
        let act = act.to_string();
        slint::Timer::single_shot(std::time::Duration::from_millis(ms), move || app::with_app(move |a| a.demo_action(&act)));
    }
}
