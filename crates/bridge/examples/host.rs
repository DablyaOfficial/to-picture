//! Мини-программа для проверки посредника: запущенная с chrome-extension://… работает как native host,
//! без аргументов — сразу выходит (так её «запускает» launch_app, если окна нет).
fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if topicture_bridge::is_host_invocation(&args) {
        std::process::exit(topicture_bridge::run_native_host());
    }
}
