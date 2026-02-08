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

#[derive(Clone, serde::Serialize)]
pub struct StatusPayload {
  pub status: AppStatus,
  pub message: Option<String>,
}

pub struct ProjectorHandle {
  pub child: Child,
  pub pid: u32,
  pub hwnd: isize,
  pub original_style: isize,
}

pub struct AppState {
  pub status: AppStatus,
  pub message: Option<String>,
  pub swf_url: Option<String>,
  pub capture_stop: Option<Arc<std::sync::atomic::AtomicBool>>,
  pub projector: Option<ProjectorHandle>,
}

impl AppState {
  pub fn new() -> Self {
    Self {
      status: AppStatus::Login,
      message: None,
      swf_url: None,
      capture_stop: None,
      projector: None,
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
