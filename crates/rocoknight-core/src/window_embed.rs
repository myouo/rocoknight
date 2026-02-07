use crate::error::{CoreError, CoreResult};
use serde::{Deserialize, Serialize};
use std::time::{Duration, Instant};

pub type RawHwnd = isize;

#[derive(Debug, Copy, Clone, Serialize, Deserialize)]
pub struct EmbedRect {
    pub x: i32,
    pub y: i32,
    pub width: i32,
    pub height: i32,
}

#[cfg(feature = "windows-native")]
mod win {
    use super::*;
    use windows::Win32::Foundation::{BOOL, HWND, LPARAM};
    use windows::Win32::UI::WindowsAndMessaging::{
        EnumWindows, GetWindowLongPtrW, GetWindowThreadProcessId, IsWindowVisible, MoveWindow,
        SetParent, SetWindowLongPtrW, SetWindowPos, HWND_TOP, GWL_STYLE, SWP_FRAMECHANGED,
        SWP_NOZORDER, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_VISIBLE,
    };

    pub fn find_window_by_pid(pid: u32, timeout: Duration) -> CoreResult<RawHwnd> {
        let start = Instant::now();
        while start.elapsed() < timeout {
            if let Some(hwnd) = enum_for_pid(pid) {
                return Ok(hwnd);
            }
            std::thread::sleep(Duration::from_millis(100));
        }
        Err(CoreError::Process("projector window not found".to_string()))
    }

    fn enum_for_pid(pid: u32) -> Option<RawHwnd> {
        let mut state = EnumState { pid, found: None };
        unsafe {
            let lparam = LPARAM(&mut state as *mut _ as isize);
            EnumWindows(Some(enum_windows_proc), lparam);
        }
        state.found
    }

    struct EnumState {
        pid: u32,
        found: Option<RawHwnd>,
    }

    unsafe extern "system" fn enum_windows_proc(hwnd: HWND, lparam: LPARAM) -> BOOL {
        let state = &mut *(lparam.0 as *mut EnumState);
        let mut window_pid: u32 = 0;
        let _ = GetWindowThreadProcessId(hwnd, Some(&mut window_pid));
        if window_pid == state.pid && IsWindowVisible(hwnd).as_bool() {
            state.found = Some(hwnd.0);
            return BOOL(0);
        }
        BOOL(1)
    }

    pub fn attach_child(parent: RawHwnd, child: RawHwnd) -> CoreResult<isize> {
        unsafe {
            let parent_hwnd = HWND(parent);
            let child_hwnd = HWND(child);
            let old_style = GetWindowLongPtrW(child_hwnd, GWL_STYLE);
            let mut new_style = old_style;
            new_style &= !(WS_OVERLAPPEDWINDOW.0 as isize);
            new_style &= !(WS_POPUP.0 as isize);
            new_style |= WS_CHILD.0 as isize;
            new_style |= WS_VISIBLE.0 as isize;
            SetWindowLongPtrW(child_hwnd, GWL_STYLE, new_style);
            let _ = SetParent(child_hwnd, parent_hwnd);
            Ok(old_style)
        }
    }

    pub fn detach(child: RawHwnd, old_style: isize) -> CoreResult<()> {
        unsafe {
            let child_hwnd = HWND(child);
            let _ = SetParent(child_hwnd, HWND(0));
            SetWindowLongPtrW(child_hwnd, GWL_STYLE, old_style);
            Ok(())
        }
    }

    pub fn set_child_rect(child: RawHwnd, rect: EmbedRect) -> CoreResult<()> {
        unsafe {
            let child_hwnd = HWND(child);
            MoveWindow(
                child_hwnd,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                true,
            );
            SetWindowPos(
                child_hwnd,
                HWND_TOP,
                rect.x,
                rect.y,
                rect.width,
                rect.height,
                SWP_NOZORDER | SWP_FRAMECHANGED,
            );
            Ok(())
        }
    }
}

#[cfg(feature = "windows-native")]
pub fn find_window_by_pid(pid: u32, timeout: Duration) -> CoreResult<RawHwnd> {
    win::find_window_by_pid(pid, timeout)
}

#[cfg(not(feature = "windows-native"))]
pub fn find_window_by_pid(_pid: u32, _timeout: Duration) -> CoreResult<RawHwnd> {
    Err(CoreError::UnsupportedPlatform)
}

#[cfg(feature = "windows-native")]
pub fn attach_child(parent: RawHwnd, child: RawHwnd) -> CoreResult<isize> {
    win::attach_child(parent, child)
}

#[cfg(not(feature = "windows-native"))]
pub fn attach_child(_parent: RawHwnd, _child: RawHwnd) -> CoreResult<isize> {
    Err(CoreError::UnsupportedPlatform)
}

#[cfg(feature = "windows-native")]
pub fn detach(child: RawHwnd, old_style: isize) -> CoreResult<()> {
    win::detach(child, old_style)
}

#[cfg(not(feature = "windows-native"))]
pub fn detach(_child: RawHwnd, _old_style: isize) -> CoreResult<()> {
    Err(CoreError::UnsupportedPlatform)
}

#[cfg(feature = "windows-native")]
pub fn set_child_rect(child: RawHwnd, rect: EmbedRect) -> CoreResult<()> {
    win::set_child_rect(child, rect)
}

#[cfg(not(feature = "windows-native"))]
pub fn set_child_rect(_child: RawHwnd, _rect: EmbedRect) -> CoreResult<()> {
    Err(CoreError::UnsupportedPlatform)
}
