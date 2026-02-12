use std::sync::OnceLock;
use tauri::{AppHandle, Emitter};

static DEBUG_APP: OnceLock<AppHandle> = OnceLock::new();

pub fn init_debug(app: AppHandle) {
  let _ = DEBUG_APP.set(app);
}

pub fn debug_log(level: &str, message: &str) {
  if let Some(app) = DEBUG_APP.get() {
    let _ = app.emit("debug_log", serde_json::json!({
      "level": level,
      "message": message
    }));
  }
}

#[macro_export]
macro_rules! debug_info {
  ($($arg:tt)*) => {
    {
      let msg = format!($($arg)*);
      tracing::info!("{}", msg);
      $crate::debug::debug_log("info", &msg);
    }
  };
}

#[macro_export]
macro_rules! debug_warn {
  ($($arg:tt)*) => {
    {
      let msg = format!($($arg)*);
      tracing::warn!("{}", msg);
      $crate::debug::debug_log("warn", &msg);
    }
  };
}

#[macro_export]
macro_rules! debug_error {
  ($($arg:tt)*) => {
    {
      let msg = format!($($arg)*);
      tracing::error!("{}", msg);
      $crate::debug::debug_log("error", &msg);
    }
  };
}

#[macro_export]
macro_rules! debug_debug {
  ($($arg:tt)*) => {
    {
      let msg = format!($($arg)*);
      tracing::debug!("{}", msg);
      $crate::debug::debug_log("debug", &msg);
    }
  };
}
