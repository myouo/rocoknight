use std::sync::{Arc, Mutex};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use std::io::Write;

use tauri::{AppHandle, Manager, State};

use crate::state::{emit_status, AppState, AppStatus};

const LOG_GREEN: &str = "\x1b[32m";
const LOG_BLUE: &str = "\x1b[34m";
const LOG_RESET: &str = "\x1b[0m";

const LOGIN3_PATH_NEEDLE: &str = "/fcgi-bin/login3";
const MAX_RESPONSE_BYTES: usize = 1_500_000;
const TIMEOUT_SECS: u64 = 180;

fn debug_log(message: &str) {
  println!("{LOG_GREEN}[RocoKnight]{LOG_BLUE}[login3]{LOG_RESET} {message}");
  tracing::info!("{message}");
}

fn redact_url(url: &str) -> String {
  match url.split_once('?') {
    Some((base, _)) => format!("{base}?REDACTED"),
    None => url.to_string(),
  }
}

fn redact_tokens(text: &str) -> String {
  let keys = ["angel_uin", "angel_key", "skey", "pskey"];
  let bytes = text.as_bytes();
  let mut out = String::with_capacity(text.len());
  let mut i = 0usize;
  while i < bytes.len() {
    let mut matched: Option<&str> = None;
    for key in keys {
      let kb = key.as_bytes();
      if bytes[i..].starts_with(kb) && i + kb.len() < bytes.len() && bytes[i + kb.len()] == b'=' {
        matched = Some(key);
        break;
      }
    }
    if let Some(key) = matched {
      out.push_str(key);
      out.push('=');
      i += key.len() + 1;
      while i < bytes.len() {
        let b = bytes[i];
        if b == b'&' || b == b'"' || b == b'\'' || b == b'<' || b == b'>' {
          break;
        }
        i += 1;
      }
      out.push('*');
      continue;
    }
    out.push(bytes[i] as char);
    i += 1;
  }
  out
}

fn sample_response(html: &str) -> String {
  let max_len = 600;
  let mut sample = html.replace('\r', " ").replace('\n', " ");
  if sample.len() > max_len {
    sample.truncate(max_len);
  }
  redact_tokens(&sample)
}

fn redact_value(value: &str) -> String {
  redact_tokens(value)
}

fn redact_swf_url(url: &str) -> String {
  if let Some((base, query)) = url.split_once('?') {
    let redacted = redact_tokens(query);
    return format!("{base}?{redacted}");
  }
  url.to_string()
}

fn maybe_dump_response(html: &str) {
  let ok = std::env::var("ROCO_DEBUG_DUMP_LOGIN3")
    .map(|v| v == "1" || v.eq_ignore_ascii_case("true"))
    .unwrap_or(false);
  if !ok {
    return;
  }
  #[cfg(target_os = "windows")]
  {
    if let Ok(local) = std::env::var("LOCALAPPDATA") {
      let path = std::path::PathBuf::from(local)
        .join("RocoKnight")
        .join("logs")
        .join("login3_dump.html");
      if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
      }
      if let Ok(mut file) = std::fs::OpenOptions::new().create(true).write(true).truncate(true).open(&path) {
        let _ = file.write_all(html.as_bytes());
        debug_log(&format!("login3 response dumped to {}", path.display()));
      }
    }
  }
}

pub fn start(app: AppHandle, state: State<Mutex<AppState>>) -> Result<(), String> {
  stop_inner(&state);

  let stop_flag = Arc::new(AtomicBool::new(false));
  with_state(&state, |s| {
    s.status = AppStatus::Capturing;
    s.message = Some("Capturing login3 response".to_string());
    s.swf_url = None;
    s.capture_stop = Some(stop_flag.clone());
  });
  emit_status(&app, &state.lock().expect("state lock"));

  start_timeout(app, stop_flag);
  debug_log("capture started");
  Ok(())
}

pub fn stop(app: AppHandle, state: State<Mutex<AppState>>) {
  stop_inner(&state);
  with_state(&state, |s| {
    s.status = AppStatus::Login;
    s.message = None;
    s.swf_url = None;
  });
  emit_status(&app, &state.lock().expect("state lock"));
  debug_log("capture stopped");
}

fn stop_inner(state: &State<Mutex<AppState>>) {
  with_state(state, |s| {
    if let Some(stop) = &s.capture_stop {
      stop.store(true, Ordering::Relaxed);
    }
    s.capture_stop = None;
  });
}

fn start_timeout(app: AppHandle, stop_flag: Arc<AtomicBool>) {
  std::thread::spawn(move || {
    let deadline = std::time::Instant::now() + Duration::from_secs(TIMEOUT_SECS);
    while std::time::Instant::now() < deadline {
      if stop_flag.load(Ordering::Relaxed) {
        return;
      }
      std::thread::sleep(Duration::from_millis(250));
    }
    if stop_flag.load(Ordering::Relaxed) {
      return;
    }
    {
      let state = app.state::<Mutex<AppState>>();
      if let Ok(mut guard) = state.lock() {
        if matches!(guard.status, AppStatus::Capturing) && guard.swf_url.is_none() {
          guard.status = AppStatus::Error;
          guard.message = Some("Login timed out (180s). Please retry.".to_string());
          guard.swf_url = None;
          emit_status(&app, &guard);
        }
      };
    }
  });
}

pub fn handle_login3_response(app: &AppHandle, state: &State<Mutex<AppState>>, html: &str) {
  let Some(value) = parse_login3_value(html) else {
    debug_log("login3 response parsed: flashVars not found; sample follows");
    debug_log(&sample_response(html));
    maybe_dump_response(html);
    return;
  };

  if !value.contains("config=") || !value.contains("angel_uin=") {
    debug_log(&format!(
      "login3 response parsed: missing required params; value sample: {}",
      sample_response(&value)
    ));
    maybe_dump_response(html);
    return;
  }

  let Some(swf_url) = build_swf_url(&value) else {
    debug_log("login3 response parsed: failed to build swf url");
    return;
  };
  debug_log(&format!(
    "flashVars captured (redacted): {}",
    redact_value(&value)
  ));
  debug_log(&format!(
    "swf url (redacted): {}",
    redact_swf_url(&swf_url)
  ));

  let should_emit = with_state(state, |s| {
    if matches!(s.status, AppStatus::Running) {
      return false;
    }
    if s.swf_url.is_some() {
      return false;
    }
    s.swf_url = Some(swf_url);
    s.status = AppStatus::FoundValue;
    s.message = Some("Found login3 value".to_string());
    true
  });

  if should_emit {
    debug_log("login3 response parsed: value accepted, moving to launch");
    emit_status(app, &state.lock().expect("state lock"));
    with_state(state, |s| {
      s.status = AppStatus::Launching;
      s.message = None;
    });
    emit_status(app, &state.lock().expect("state lock"));
    stop_timer_only(state);
    let app_handle = app.clone();
    let _ = app_handle.clone().run_on_main_thread(move || {
      let state_handle = app_handle.state::<Mutex<AppState>>();
      let _ = crate::launcher::launch_projector_auto(&app_handle, &state_handle);
    });
  }
}

pub fn stop_timer_only(state: &State<Mutex<AppState>>) {
  with_state(state, |s| {
    if let Some(stop) = &s.capture_stop {
      stop.store(true, Ordering::Relaxed);
    }
  });
}

pub fn parse_login3_value(html: &str) -> Option<String> {
  let needle = "function swf";
  let start = html.find(needle)?;
  let slice = &html[start..];
  if let Some(value) = extract_attr_value(slice, "flashVars").or_else(|| extract_attr_value(slice, "FlashVars")) {
    return Some(value);
  }
  if let Some(value) = extract_attr_value(html, "flashVars").or_else(|| extract_attr_value(html, "FlashVars")) {
    return Some(value);
  }
  let unescaped = unescape_source(html);
  extract_attr_value(&unescaped, "flashVars").or_else(|| extract_attr_value(&unescaped, "FlashVars"))
}

fn extract_attr_value(text: &str, attr: &str) -> Option<String> {
  let bytes = text.as_bytes();
  let attr_bytes = attr.as_bytes();
  let mut i = 0usize;
  while i + attr_bytes.len() < bytes.len() {
    if starts_with_ignore_case(&bytes[i..], attr_bytes) {
      let mut j = i + attr_bytes.len();
      while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
      }
      if j >= bytes.len() || bytes[j] != b'=' {
        i += 1;
        continue;
      }
      j += 1;
      while j < bytes.len() && bytes[j].is_ascii_whitespace() {
        j += 1;
      }
      if j >= bytes.len() {
        break;
      }
      let mut quote = bytes[j];
      if quote == b'\\' && j + 1 < bytes.len() {
        let next = bytes[j + 1];
        if next == b'"' || next == b'\'' {
          quote = next;
          j += 1;
        }
      }
      if quote != b'"' && quote != b'\'' {
        i += 1;
        continue;
      }
      j += 1;
      let mut out = String::new();
      let mut escape = false;
      while j < bytes.len() {
        let ch = bytes[j] as char;
        if escape {
          match ch {
            'n' => out.push('\n'),
            'r' => out.push('\r'),
            't' => out.push('\t'),
            '\\' => out.push('\\'),
            '\'' => out.push('\''),
            '"' => out.push('"'),
            _ => out.push(ch),
          }
          escape = false;
          j += 1;
          continue;
        }
        if ch == '\\' {
          escape = true;
          j += 1;
          continue;
        }
        if ch as u8 == quote {
          return Some(out);
        }
        out.push(ch);
        j += 1;
      }
      return None;
    }
    i += 1;
  }
  None
}

fn starts_with_ignore_case(hay: &[u8], needle: &[u8]) -> bool {
  if hay.len() < needle.len() {
    return false;
  }
  for (a, b) in hay.iter().take(needle.len()).zip(needle.iter()) {
    if a.to_ascii_lowercase() != b.to_ascii_lowercase() {
      return false;
    }
  }
  true
}

fn unescape_source(text: &str) -> String {
  let mut out = String::with_capacity(text.len());
  let mut chars = text.chars().peekable();
  while let Some(ch) = chars.next() {
    if ch == '\\' {
      if let Some(next) = chars.next() {
        match next {
          'n' => out.push('\n'),
          'r' => out.push('\r'),
          't' => out.push('\t'),
          '"' => out.push('"'),
          '\'' => out.push('\''),
          '\\' => out.push('\\'),
          other => {
            out.push('\\');
            out.push(other);
          }
        }
        continue;
      }
    }
    out.push(ch);
  }
  out
}

fn build_swf_url(value: &str) -> Option<String> {
  let trimmed = value.trim().trim_start_matches('?').trim_start_matches('&');
  if trimmed.is_empty() {
    return None;
  }
  let nonce = build_nonce_key();
  Some(format!(
    "https://res.17roco.qq.com/main.swf?{nonce}=&{trimmed}"
  ))
}

fn build_nonce_key() -> String {
  let nanos = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_else(|_| Duration::from_secs(0))
    .subsec_nanos();
  let fraction = (nanos as f64) / 1_000_000_000.0;
  format!("{fraction:.16}")
}

fn with_state<R>(state: &State<Mutex<AppState>>, f: impl FnOnce(&mut AppState) -> R) -> R {
  let mut guard = state.lock().expect("state lock");
  f(&mut guard)
}

#[cfg(windows)]
pub fn attach_webview2_capture(webview: tauri::webview::PlatformWebview, app: AppHandle) {
  use webview2_com::{
    take_pwstr, WebResourceResponseReceivedEventHandler,
    WebResourceResponseViewGetContentCompletedHandler,
  };
  use webview2_com::Microsoft::Web::WebView2::Win32::{
    COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL, ICoreWebView2_2,
  };
  use windows::core::{w, Interface, PWSTR};
  use windows::Win32::System::Com::IStream;

  let controller = webview.controller();
  let core = match unsafe { controller.CoreWebView2() } {
    Ok(core) => core,
    Err(_) => {
      debug_log("attach failed: CoreWebView2 not available");
      return;
    }
  };

  let _ = unsafe {
    core.AddWebResourceRequestedFilter(w!("*"), COREWEBVIEW2_WEB_RESOURCE_CONTEXT_ALL)
  };
  debug_log("attach ok: WebResourceRequestedFilter added");

  let app_handle = app.clone();
  let response_handler = WebResourceResponseReceivedEventHandler::create(Box::new(
    move |_webview, args| {
      let Some(args) = args else {
        return Ok(());
      };
      let request = unsafe { args.Request() }?;
      let mut uri_pw = PWSTR::null();
      unsafe { request.Uri(&mut uri_pw) }?;
      let url = take_pwstr(uri_pw);
      let url_lc = url.to_ascii_lowercase();
      if !url_lc.contains(LOGIN3_PATH_NEEDLE) {
        return Ok(());
      }
      debug_log(&format!(
        "login3 response event: {}",
        redact_url(&url)
      ));
      let response = unsafe { args.Response() }?;
      let app_for_content = app_handle.clone();
      let handler = WebResourceResponseViewGetContentCompletedHandler::create(Box::new(
        move |result, stream: Option<IStream>| {
          if result.is_err() {
            debug_log("login3 response GetContent failed");
            return Ok(());
          }
          let Some(stream) = stream else {
            debug_log("login3 response GetContent empty stream");
            return Ok(());
          };
          let html = read_stream_to_string(&stream, MAX_RESPONSE_BYTES);
          if let Some(html) = html {
            debug_log(&format!("login3 response size: {} bytes", html.len()));
            let state = app_for_content.state::<Mutex<AppState>>();
            handle_login3_response(&app_for_content, &state, &html);
          } else {
            debug_log("login3 response read_stream_to_string failed");
          }
          Ok(())
        },
      ));
      let _ = unsafe { response.GetContent(&handler) };
      Ok(())
    },
  ));

  let core2: ICoreWebView2_2 = match core.cast() {
    Ok(core2) => core2,
    Err(_) => {
      return;
    }
  };
  let mut token: i64 = 0;
  let _ = unsafe { core2.add_WebResourceResponseReceived(&response_handler, &mut token) };
  std::mem::forget(response_handler);
  debug_log("attach ok: WebResourceResponseReceived handler registered");
}

#[cfg(not(windows))]
pub fn attach_webview2_capture(_webview: tauri::webview::PlatformWebview, _app: AppHandle) {}

#[cfg(windows)]
fn read_stream_to_string(stream: &windows::Win32::System::Com::IStream, limit: usize) -> Option<String> {
  let mut buf = Vec::new();
  let mut chunk = [0u8; 4096];
  let mut total = 0usize;
  loop {
    let mut read = 0u32;
    let hr = unsafe {
      stream.Read(
        chunk.as_mut_ptr() as *mut _,
        chunk.len() as u32,
        Some(&mut read),
      )
    };
    if hr.is_err() {
      return None;
    }
    if read == 0 {
      break;
    }
    let take = read as usize;
    let remaining = limit.saturating_sub(total);
    if remaining == 0 {
      break;
    }
    let slice_len = take.min(remaining);
    buf.extend_from_slice(&chunk[..slice_len]);
    total += slice_len;
    if total >= limit {
      break;
    }
  }
  Some(String::from_utf8_lossy(&buf).to_string())
}

#[cfg(test)]
mod tests {
  use super::parse_login3_value;

  #[test]
  fn parse_value_from_script() {
    let html = r#"
    <html><body>
    <script>function swf(id){
      var swfurl='<param name="FlashVars" value="config=//res.17roco.qq.com/Global.xml&angel_uin=123&angel_key=abc&skey=def&pskey=ghi" />';
    }</script>
    </body></html>
    "#;
    let value = parse_login3_value(html).expect("value should be found");
    assert!(value.contains("config="));
    assert!(value.contains("angel_uin="));
  }

  #[test]
  fn parse_value_with_escaped_quotes() {
    let html = r#"function swf(id){var swfurl="<embed flashVars=\"config=//res.17roco.qq.com/Global.xml&angel_uin=1\" />";}"#;
    let value = parse_login3_value(html).expect("value should be found");
    assert!(value.contains("angel_uin=1"));
  }
}
