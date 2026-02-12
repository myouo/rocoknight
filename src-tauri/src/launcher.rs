use std::sync::Mutex;
use std::time::Duration;
use std::sync::Arc;

use tauri::{AppHandle, Manager, State};
use tauri::PhysicalSize;
use windows::Win32::Foundation::HWND;

use crate::embed_win32::{
  attach_child,
  bring_to_top,
  detach_child,
  find_window_by_pid,
  hide_window,
  move_child,
  parent_client_size,
};
use crate::projector::{resolve_projector_path, stop_projector as kill_projector};
use crate::state::{emit_status, AppState, AppStatus, ProjectorHandle};
use crate::wpe::{PacketInjector, PacketInterceptor};
use tracing::info;

const LOGIN_ZOOM: f64 = 1.17;
const UI_BAR_HEIGHT: i32 = 36;

fn extract_qq_from_url(url: &str) -> Option<u64> {
  url::Url::parse(url).ok()
    .and_then(|parsed| {
      parsed.query_pairs()
        .find(|(key, _)| key == "qq" || key == "uin")
        .and_then(|(_, value)| value.parse::<u64>().ok())
    })
}

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

fn main_window_scale(app: &AppHandle) -> f64 {
  if let Ok(window) = main_window(app) {
    if let Ok(scale) = window.scale_factor() {
      return scale;
    }
  }
  1.0
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
      kill_projector(&mut projector.process);
    }

    if let Some(interceptor) = s.wpe_interceptor.take() {
      info!("[WPE] Stopping interceptor");
      interceptor.stop();
    }

    s.status = AppStatus::Login;
    s.message = None;
    s.last_projector_rect = None;
    s.qq_num = None;
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
  let process = match crate::projector::launch_projector(&projector_path, &swf_url) {
    Ok(process) => process,
    Err(msg) => {
      set_error(app, state, msg.clone());
      return Err(msg);
    }
  };
  let pid = process.pid;

  let child_hwnd = match find_window_by_pid(pid, 6000) {
    Ok(hwnd) => hwnd,
    Err(msg) => {
      set_error(app, state, msg.clone());
      return Err(msg);
    }
  };
  hide_window(child_hwnd);
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

  if let Some((w, h)) = parent_client_size(main_hwnd) {
    let scale = main_window_scale(app);
    let bar_h = ((UI_BAR_HEIGHT as f64) * scale).round() as i32;
    let usable_h = (h - bar_h).max(1);
    move_child(child_hwnd, 0, bar_h, w, usable_h);
  } else {
    let size = main_window_size_physical(app)?;
    let scale = main_window_scale(app);
    let bar_h = ((UI_BAR_HEIGHT as f64) * scale).round() as i32;
    let usable_h = (size.height as i32 - bar_h).max(1);
    move_child(child_hwnd, 0, bar_h, size.width as i32, usable_h);
  }
  bring_to_top(child_hwnd);
  info!("[RocoKnight][launcher] projector attached and brought to top");
  schedule_projector_fit(app.clone());

  let qq_num = extract_qq_from_url(&swf_url).unwrap_or(0);
  info!("[RocoKnight][launcher] extracted QQ number: {}", qq_num);

  let injector = match PacketInjector::new(pid) {
    Ok(inj) => Arc::new(inj),
    Err(e) => {
      info!("[WPE] Failed to create injector: {}", e);
      return Err(format!("Failed to create packet injector: {}", e));
    }
  };

  let interceptor = match PacketInterceptor::new(pid) {
    Ok(int) => int,
    Err(e) => {
      info!("[WPE] Failed to create interceptor: {}", e);
      return Err(format!("Failed to create packet interceptor: {}", e));
    }
  };

  with_state(state, |s| {
    s.projector = Some(ProjectorHandle {
      process,
      hwnd: child_hwnd.0 as isize,
      original_style,
    });
    s.status = AppStatus::Running;
    s.message = None;
    s.last_projector_rect = None;
    s.qq_num = Some(qq_num);
    s.wpe_interceptor = Some(interceptor);
  });
  emit_status(app, &state.lock().expect("state lock"));

  if let Some(login) = app.get_webview("login") {
    let _ = login.hide();
  }
  if let Some(main) = app.get_webview("main") {
    let _ = main.hide();
  }
  info!("[RocoKnight][launcher] login webview hidden for projector");

  Ok(())
}

fn schedule_projector_fit(app: AppHandle) {
  std::thread::spawn(move || {
    let delays_ms = [50u64, 150, 300, 600, 1200, 2000];
    for delay in delays_ms {
      std::thread::sleep(Duration::from_millis(delay));
      let app_clone = app.clone();
      let app_for_task = app_clone.clone();
      let _ = app_clone.run_on_main_thread(move || {
        let state = app_for_task.state::<Mutex<AppState>>();
        resize_projector_to_window(&app_for_task, &state);
      });
    }
  });
}

pub fn resize_projector_to_window(app: &AppHandle, state: &State<Mutex<AppState>>) {
  let (projector, last_rect) = with_state(state, |s| {
    (s.projector.as_ref().map(|p| p.hwnd), s.last_projector_rect)
  });
  let Some(hwnd) = projector else {
    return;
  };

  let rect = if let Ok(parent) = main_hwnd(app) {
    if let Some((w, h)) = parent_client_size(parent) {
      let scale = main_window_scale(app);
      let bar_h = ((UI_BAR_HEIGHT as f64) * scale).round() as i32;
      let usable_h = (h - bar_h).max(1);
      Some((0, bar_h, w, usable_h))
    } else {
      None
    }
  } else {
    None
  }
  .or_else(|| {
    main_window_size_physical(app).ok().map(|size| {
      let scale = main_window_scale(app);
      let bar_h = ((UI_BAR_HEIGHT as f64) * scale).round() as i32;
      let usable_h = (size.height as i32 - bar_h).max(1);
      (0, bar_h, size.width as i32, usable_h)
    })
  });

  let Some((x, y, w, h)) = rect else {
    return;
  };
  if Some((x, y, w, h)) == last_rect {
    return;
  }

  move_child(HWND(hwnd as *mut std::ffi::c_void), x, y, w, h);
  bring_to_top(HWND(hwnd as *mut std::ffi::c_void));
  with_state(state, |s| {
    s.last_projector_rect = Some((x, y, w, h));
  });
}

pub fn resize_login_to_window(app: &AppHandle) {
  if let Ok(window) = main_window(app) {
    if let Ok(size) = window.inner_size() {
      let scale = window.scale_factor().unwrap_or(1.0);
      let w = ((size.width as f64) / scale).round() as i32;
      let h = ((size.height as f64) / scale).round() as i32;
      if let Some(login) = app.get_webview("login") {
        let usable_h = (h - UI_BAR_HEIGHT).max(1);
        let _ = login.set_position(tauri::LogicalPosition::new(0, UI_BAR_HEIGHT));
        let _ = login.set_size(tauri::LogicalSize::new(w, usable_h));
        let _ = login.set_zoom(LOGIN_ZOOM);
      }
      if let Some(toolbar) = app.get_webview("toolbar") {
        let _ = toolbar.set_position(tauri::LogicalPosition::new(0, 0));
        let _ = toolbar.set_size(tauri::LogicalSize::new(w, UI_BAR_HEIGHT));
      }
    }
  }
}

pub fn schedule_login_layout(app: AppHandle) {
  std::thread::spawn(move || {
    let delays_ms = [50u64, 150, 300, 600];
    for delay in delays_ms {
      std::thread::sleep(Duration::from_millis(delay));
      let app_clone = app.clone();
      let app_for_cb = app_clone.clone();
      let _ = app_clone.run_on_main_thread(move || {
        resize_login_to_window(&app_for_cb);
      });
    }
  });
}
