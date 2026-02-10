#[cfg(target_os = "windows")]
mod win {
  use std::time::{Duration, Instant};
  use windows::core::BOOL;
  use windows::Win32::Foundation::{HWND, LPARAM};
  use windows::Win32::UI::HiDpi::{SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2};
  use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClientRect, GetWindow, GetWindowLongPtrW, GetWindowThreadProcessId,
    IsWindowVisible, MoveWindow, SetParent, SetWindowLongPtrW, SetWindowPos, ShowWindow, GWL_STYLE,
    GW_OWNER, HWND_TOP, SW_HIDE, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER,
    SWP_SHOWWINDOW, WS_CHILD, WS_MAXIMIZEBOX, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_SIZEBOX, WS_VISIBLE,
  };
  use windows::Win32::Foundation::RECT;

  #[derive(Default)]
  struct FindData {
    pid: u32,
    hwnd: HWND,
  }

  unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
    let data = &mut *(lparam.0 as *mut FindData);
    let mut window_pid: u32 = 0;
    GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
    if window_pid != data.pid {
      return BOOL(1);
    }
    let owner = GetWindow(hwnd, GW_OWNER).unwrap_or(HWND(std::ptr::null_mut()));
    if owner.0.is_null() && IsWindowVisible(hwnd).as_bool() {
      data.hwnd = hwnd;
      return BOOL(0);
    }
    if owner.0.is_null() {
      data.hwnd = hwnd;
      return BOOL(0);
    }
    BOOL(1)
  }

  pub fn find_window_by_pid(pid: u32, timeout_ms: u64) -> Result<HWND, String> {
    let start = Instant::now();
    loop {
      let mut data = FindData { pid, hwnd: HWND(std::ptr::null_mut()) };
      unsafe {
        let _ = EnumWindows(Some(enum_windows_proc), LPARAM(&mut data as *mut _ as isize));
      }
      if !data.hwnd.0.is_null() {
        return Ok(data.hwnd);
      }
      if start.elapsed() > Duration::from_millis(timeout_ms) {
        return Err("未能在超时内找到 projector 窗口。".to_string());
      }
      std::thread::sleep(Duration::from_millis(100));
    }
  }

  pub fn attach_child(child_hwnd: HWND, parent_hwnd: HWND) -> Result<isize, String> {
    unsafe {
      let original_style = GetWindowLongPtrW(child_hwnd, GWL_STYLE);
      let mut new_style = original_style;
      new_style &= !(WS_OVERLAPPEDWINDOW.0 as isize | WS_POPUP.0 as isize);
      new_style |= (WS_CHILD.0 | WS_VISIBLE.0) as isize;
      let _ = SetParent(child_hwnd, Some(parent_hwnd));
      SetWindowLongPtrW(child_hwnd, GWL_STYLE, new_style);
      let _ = SetWindowPos(
        child_hwnd,
        None,
        0,
        0,
        1,
        1,
        SWP_FRAMECHANGED | SWP_NOZORDER | SWP_SHOWWINDOW,
      );
      Ok(original_style)
    }
  }

  pub fn detach_child(child_hwnd: HWND, original_style: isize) {
    unsafe {
      let _ = SetParent(child_hwnd, None);
      SetWindowLongPtrW(child_hwnd, GWL_STYLE, original_style);
      let _ = SetWindowPos(
        child_hwnd,
        None,
        0,
        0,
        1,
        1,
        SWP_FRAMECHANGED | SWP_NOZORDER | SWP_SHOWWINDOW,
      );
    }
  }

  pub fn move_child(child_hwnd: HWND, x: i32, y: i32, w: i32, h: i32) {
    unsafe {
      let _ = MoveWindow(child_hwnd, x, y, w, h, true);
    }
  }

  pub fn set_dpi_awareness() -> bool {
    unsafe { SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2).is_ok() }
  }

  pub fn disable_maximize_resize(hwnd: HWND) {
    unsafe {
      let mut style = GetWindowLongPtrW(hwnd, GWL_STYLE);
      style &= !(WS_MAXIMIZEBOX.0 as isize);
      style &= !(WS_SIZEBOX.0 as isize);
      SetWindowLongPtrW(hwnd, GWL_STYLE, style);
      let _ = SetWindowPos(
        hwnd,
        Some(HWND_TOP),
        0,
        0,
        0,
        0,
        SWP_FRAMECHANGED | SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER,
      );
    }
  }

  pub fn parent_client_size(parent_hwnd: HWND) -> Option<(i32, i32)> {
    unsafe {
      let mut rect = RECT::default();
      if GetClientRect(parent_hwnd, &mut rect).is_ok() {
        let w = rect.right - rect.left;
        let h = rect.bottom - rect.top;
        if w > 0 && h > 0 {
          return Some((w, h));
        }
      }
    }
    None
  }

  pub fn bring_to_top(child_hwnd: HWND) {
    unsafe {
      let _ = SetWindowPos(
        child_hwnd,
        Some(HWND_TOP),
        0,
        0,
        0,
        0,
        SWP_FRAMECHANGED | SWP_SHOWWINDOW | SWP_NOMOVE | SWP_NOSIZE,
      );
    }
  }

  pub fn hide_window(child_hwnd: HWND) {
    unsafe {
      let _ = ShowWindow(child_hwnd, SW_HIDE);
    }
  }
}

#[cfg(target_os = "windows")]
pub use win::*;

#[cfg(not(target_os = "windows"))]
mod non_win {
  use windows::Win32::Foundation::HWND;

  pub fn find_window_by_pid(_pid: u32, _timeout_ms: u64) -> Result<HWND, String> {
    Err("仅支持 Windows 平台。".to_string())
  }

  pub fn attach_child(_child_hwnd: HWND, _parent_hwnd: HWND) -> Result<isize, String> {
    Err("仅支持 Windows 平台。".to_string())
  }

  pub fn detach_child(_child_hwnd: HWND, _original_style: isize) {}

  pub fn move_child(_child_hwnd: HWND, _x: i32, _y: i32, _w: i32, _h: i32) {}

  pub fn set_dpi_awareness() -> bool {
    false
  }

  pub fn disable_maximize_resize(_hwnd: HWND) {}

  pub fn parent_client_size(_parent_hwnd: HWND) -> Option<(i32, i32)> {
    None
  }

  pub fn bring_to_top(_child_hwnd: HWND) {}

  pub fn hide_window(_child_hwnd: HWND) {}
}

#[cfg(not(target_os = "windows"))]
pub use non_win::*;
