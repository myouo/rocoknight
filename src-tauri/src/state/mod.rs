use crate::speed::SpeedShmem;
use std::process::Child;
use std::sync::Arc;
use tauri::{AppHandle, Emitter};

#[derive(Clone, serde::Serialize)]
pub enum AppStatus {
    Login,
    Capturing,
    FoundValue,
    Launching,
    Running,
    Error,
}

#[derive(Clone, Copy, serde::Serialize)]
pub enum ThemeMode {
    Dark,
    Light,
}

impl ThemeMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThemeMode::Dark => "dark",
            ThemeMode::Light => "light",
        }
    }
}

#[derive(Clone, serde::Serialize)]
pub struct StatusPayload {
    pub status: AppStatus,
    pub message: Option<String>,
}

pub struct ProjectorHandle {
    pub child: Child,
    pub hwnd: isize,
    pub original_style: isize,
}

pub struct AppState {
    pub status: AppStatus,
    pub message: Option<String>,
    pub theme_mode: ThemeMode,
    pub swf_url: Option<String>,
    pub capture_stop: Option<Arc<std::sync::atomic::AtomicBool>>,
    pub projector: Option<ProjectorHandle>,
    pub speed_multiplier: f64,
    pub speed_shmem: Option<SpeedShmem>,
}

impl AppState {
    pub fn new() -> Self {
        Self {
            status: AppStatus::Login,
            message: None,
            theme_mode: ThemeMode::Dark,
            swf_url: None,
            capture_stop: None,
            projector: None,
            speed_multiplier: 1.0,
            speed_shmem: None,
        }
    }
}

pub fn emit_status(app: &AppHandle, state: &AppState) {
    let payload = StatusPayload {
        status: state.status.clone(),
        message: state.message.clone(),
    };
    let _ = app.emit("status_changed", payload);
}
