#[cfg(feature = "windows-native")]
use windows::Win32::Foundation::HWND;

#[cfg(feature = "windows-native")]
pub fn set_window_title(_hwnd: HWND, _title: &str) {
    // TODO: implement with SetWindowTextW
}

#[cfg(not(feature = "windows-native"))]
pub fn set_window_title(_hwnd: usize, _title: &str) {
    // no-op on non-windows builds
}

