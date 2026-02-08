use std::fs;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};

use tauri::{AppHandle, Manager};
use tauri::path::BaseDirectory;
use tracing::{error, info};
use url::Url;

pub fn resolve_projector_path(app: &AppHandle) -> Result<PathBuf, String> {
  let resolved = app
    .path()
    .resolve("projector.exe", BaseDirectory::Resource)
    .map_err(|_| "Failed to resolve resource directory.".to_string())?;
  if fs::metadata(&resolved).is_ok() {
    info!("projector path resolved: {}", resolved.display());
    return Ok(resolved);
  }

  let resource_dir = app
    .path()
    .resource_dir()
    .map_err(|_| "Failed to get resource directory.".to_string())?;
  let fallback = resource_dir.join("projector.exe");
  if fs::metadata(&fallback).is_ok() {
    info!("projector path resolved (fallback): {}", fallback.display());
    return Ok(fallback);
  }

  Err(format!(
    "Failed to locate projector.exe. Checked: {}, {}.",
    resolved.display(),
    fallback.display()
  ))
}

pub fn launch_projector(path: &PathBuf, swf_url: &str) -> Result<Child, String> {
  info!("launching projector: {} {}", path.display(), sanitize_url_for_log(swf_url));
  let child = Command::new(path)
    .arg(swf_url)
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .spawn()
    .map_err(|err| {
      error!("launch projector failed: {err}");
      "Failed to launch projector.".to_string()
    })?;
  Ok(child)
}

pub fn stop_projector(child: &mut Child) {
  let _ = child.kill();
  let _ = child.wait();
}

fn sanitize_url_for_log(url: &str) -> String {
  let Ok(parsed) = Url::parse(url) else {
    return "<invalid-url>".to_string();
  };
  let origin = parsed.origin().ascii_serialization();
  let path = parsed.path();
  let mut keys: Vec<String> = Vec::new();
  for (k, _v) in parsed.query_pairs() {
    keys.push(k.to_string());
  }
  keys.sort();
  keys.dedup();
  if keys.is_empty() {
    format!("{origin}{path}")
  } else {
    format!("{origin}{path} ?keys={}", keys.join(","))
  }
}
