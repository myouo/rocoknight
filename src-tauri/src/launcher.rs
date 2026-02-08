use std::sync::Mutex;

use tauri::{AppHandle, Manager, State};
use tauri::{PhysicalSize, Size};
use windows::Win32::Foundation::HWND;

use crate::embed_win32::{attach_child, bring_to_top, detach_child, find_window_by_pid, move_child};
use crate::projector::{resolve_projector_path, stop_projector as kill_projector};
use crate::state::{emit_status, AppState, AppStatus, ProjectorHandle};
use tracing::info;

fn main_window(app: &AppHandle) -> Result<tauri::Window, String> {
  app
    .get_window("main")
    .ok_or_else(|| "Main window not found.".to_string())
}

fn main_hwnd(app: &AppHandle) -> Result<HWND, String> {
  let window = main_window(app)?;
  window
    .hwnd()
    .map_err(|_| "Failed to get main window handle.".to_string())
}

fn main_window_size_physical(app: &AppHandle) -> Result<PhysicalSize<u32>, String> {
  let window = main_window(app)?;
  let size = window
    .inner_size()
    .map_err(|_| "Failed to get window size.".to_string())?;
  Ok(size)
}

fn with_state<R>(state: &State<Mutex<AppState>>, f: impl FnOnce(&mut AppState) -> R) -> R {
  let mut guard = state.lock().expect("state lock");
  f(&mut guard)
}

fn set_error(app: &AppHandle, state: &State<Mutex<AppState>>, msg: String) {
  with_state(state, |s| {
    s.status = AppStatus::Error;
    s.message = Some(msg.clone());
  });
  emit_status(app, &state.lock().expect("state lock"));
}

pub fn stop_projector(state: &State<Mutex<AppState>>) {
  with_state(state, |s| {
    if let Some(mut projector) = s.projector.take() {
      detach_child(HWND(projector.hwnd as *mut std::ffi::c_void), projector.original_style);
      kill_projector(&mut projector.child);
    }
    s.status = AppStatus::Login;
    s.message = None;
    s.swf_url = None;
  });
}

pub fn launch_projector_auto(app: &AppHandle, state: &State<Mutex<AppState>>) -> Result<(), String> {
  let (swf_url, existing) = with_state(state, |s| (s.swf_url.clone(), s.projector.is_some()));
  if existing {
    stop_projector(state);
  }
  let swf_url = match swf_url {
    Some(url) => url,
    None => {
      let msg = "Missing main.swf URL.".to_string();
      set_error(app, state, msg.clone());
      return Err(msg);
    }
  };

  let projector_path = match resolve_projector_path(app) {
    Ok(path) => path,
    Err(msg) => {
      set_error(app, state, msg.clone());
      return Err(msg);
    }
  };
  let child = match crate::projector::launch_projector(&projector_path, &swf_url) {
    Ok(child) => child,
    Err(msg) => {
      set_error(app, state, msg.clone());
      return Err(msg);
    }
  };
  let pid = child.id();

  let child_hwnd = match find_window_by_pid(pid, 6000) {
    Ok(hwnd) => hwnd,
    Err(msg) => {
      set_error(app, state, msg.clone());
      return Err(msg);
    }
  };
  let main_hwnd = match main_hwnd(app) {
    Ok(hwnd) => hwnd,
    Err(msg) => {
      set_error(app, state, msg.clone());
      return Err(msg);
    }
  };
  let original_style = match attach_child(child_hwnd, main_hwnd) {
    Ok(style) => style,
    Err(msg) => {
      set_error(app, state, msg.clone());
      return Err(msg);
    }
  };

  let size = main_window_size_physical(app)?;
  move_child(child_hwnd, 0, 0, size.width as i32, size.height as i32);
  bring_to_top(child_hwnd);
  info!("[RocoKnight][launcher] projector attached and brought to top");

  with_state(state, |s| {
    s.projector = Some(ProjectorHandle {
      child,
      pid,
      hwnd: child_hwnd.0 as isize,
      original_style,
    });
    s.status = AppStatus::Running;
    s.message = None;
    s.swf_url = None;
  });
  emit_status(app, &state.lock().expect("state lock"));

  if let Some(login) = app.get_webview("login") {
    let _ = login.hide();
  }
  if let Some(main_webview) = app.get_webview("main") {
    let _ = main_webview.hide();
  }
  info!("[RocoKnight][launcher] webviews hidden for projector");

  Ok(())
}

pub fn resize_projector_to_window(app: &AppHandle, state: &State<Mutex<AppState>>) {
  let projector = with_state(state, |s| s.projector.as_ref().map(|p| p.hwnd));
  if let Some(hwnd) = projector {
    if let Ok(size) = main_window_size_physical(app) {
      move_child(
        HWND(hwnd as *mut std::ffi::c_void),
        0,
        0,
        size.width as i32,
        size.height as i32,
      );
      bring_to_top(HWND(hwnd as *mut std::ffi::c_void));
    }
  }
}

pub fn resize_login_to_window(app: &AppHandle) {
  if let Ok(window) = main_window(app) {
    if let Ok(size) = window.inner_size() {
      let scale = window.scale_factor().unwrap_or(1.0);
      let w = ((size.width as f64) / scale).round() as i32;
      let h = ((size.height as f64) / scale).round() as i32;
      if let Some(login) = app.get_webview("login") {
        let _ = login.set_position(tauri::LogicalPosition::new(0, 0));
        let _ = login.set_size(tauri::LogicalSize::new(w, h));
      }
    }
  }
}

pub fn ensure_main_size_ratio(app: &AppHandle, size: PhysicalSize<u32>) {
  let window = match main_window(app) {
    Ok(w) => w,
    Err(_) => return,
  };
  let target_w = size.width;
  let target_h = size.height;
  if target_h == 0 || target_w == 0 {
    return;
  }
  let desired_h = target_w * 3 / 4;
  if target_h != desired_h {
    let _ = window.set_size(Size::Physical(PhysicalSize::new(target_w, desired_h)));
  }
}
