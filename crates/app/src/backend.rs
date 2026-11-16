//! Связь окна с движком и расширением. Один интерфейс — для настоящего движка (topicture_core::Engine +
//! topicture_bridge) и для демо-режима (TOPICTURE_DEMO=1), чтобы весь код окна был общим.
use std::collections::HashMap;
use std::sync::Arc;
use topicture_bridge as bridge;
use topicture_core::{Engine, EventSink, GpuInfo, InitInfo, Item, Override, PreviewResult, Settings};

pub trait Backend: Send + Sync {
    /// Быстро: настройки, отсутствующие программы, язык.
    fn init(&self) -> InitInfo;
    fn save_settings(&self, s: &Settings);
    /// Блокирует (до минут) — только из фонового потока.
    fn preview(&self, url: &str) -> PreviewResult;
    fn start(&self, items: Vec<Item>, settings: Settings, overrides: HashMap<String, Override>) -> bool;
    fn pause(&self, on: bool);
    fn cancel(&self);
    /// Блокирует до 30 с — только из фонового потока.
    fn gpu_info(&self) -> Option<GpuInfo>;
    /// Может запускать программы — из фонового потока.
    fn versions(&self) -> String;
    fn shutdown(&self);
    fn put_cookies(&self, site: String, netscape: String);

    fn ext_status(&self) -> bridge::ExtStatus;
    fn ext_register(&self) -> bool;
    fn take_inbox(&self) -> Vec<bridge::InboxLink>;
    fn should_show_help_once(&self) -> bool;
    fn start_ipc(&self, on_wake: Box<dyn Fn() + Send + Sync>);
    fn stop_ipc(&self);
}

// ---------------------------------------------------------------- настоящий движок

pub struct Real {
    engine: Arc<Engine>,
    ipc: std::sync::Mutex<Option<bridge::IpcServer>>,
}

impl Real {
    pub fn new(sink: EventSink) -> Real {
        Real { engine: Engine::new(sink), ipc: Default::default() }
    }
}

impl Backend for Real {
    fn init(&self) -> InitInfo { self.engine.init() }
    fn save_settings(&self, s: &Settings) { self.engine.save_settings(s) }
    fn preview(&self, url: &str) -> PreviewResult { self.engine.preview(url) }
    fn start(&self, items: Vec<Item>, settings: Settings, overrides: HashMap<String, Override>) -> bool {
        self.engine.start(items, settings, overrides)
    }
    fn pause(&self, on: bool) { self.engine.pause(on) }
    fn cancel(&self) { self.engine.cancel() }
    fn gpu_info(&self) -> Option<GpuInfo> { self.engine.gpu_info() }
    fn versions(&self) -> String { self.engine.versions() }
    fn shutdown(&self) { self.engine.shutdown() }
    fn put_cookies(&self, site: String, netscape: String) { self.engine.cookies.put(site, netscape) }
    fn ext_status(&self) -> bridge::ExtStatus { bridge::ext_status() }
    fn ext_register(&self) -> bool { bridge::register_extension().is_ok() && bridge::extension_registered() }
    fn take_inbox(&self) -> Vec<bridge::InboxLink> { bridge::take_inbox() }
    fn should_show_help_once(&self) -> bool { bridge::should_show_help_once() }
    fn start_ipc(&self, on_wake: Box<dyn Fn() + Send + Sync>) {
        *self.ipc.lock().unwrap() = bridge::IpcServer::start(on_wake);
    }
    fn stop_ipc(&self) {
        if let Some(s) = self.ipc.lock().unwrap().take() {
            s.stop();
        }
    }
}
