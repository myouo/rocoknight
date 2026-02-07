use crate::error::{CoreError, CoreResult};
use crate::config::CoreConfig;
use std::process::{Child, Command};
use std::sync::{Arc, Mutex};
use std::collections::HashMap;

#[derive(Debug, Clone)]
pub struct ProcessHandle {
    pub id: u64,
}

#[derive(Clone, Default)]
pub struct ProcessManager {
    inner: Arc<Mutex<ProcessState>>,
}

#[derive(Default)]
struct ProcessState {
    next_id: u64,
    children: HashMap<u64, Child>,
}

impl ProcessManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn launch_projector(&self, cfg: &CoreConfig) -> CoreResult<ProcessHandle> {
        let projector = cfg.launcher.projector_path.clone().ok_or_else(|| {
            CoreError::Config("projector_path is required".to_string())
        })?;
        let url = cfg.launcher.main_swf_url.clone().ok_or_else(|| {
            CoreError::Config("main_swf_url is required".to_string())
        })?;
        self.launch_projector_with_url(projector, url)
    }

    pub fn launch_projector_with_url(
        &self,
        projector_path: std::path::PathBuf,
        url: String,
    ) -> CoreResult<ProcessHandle> {
        let mut cmd = Command::new(projector_path);
        cmd.arg(url);
        let child = cmd.spawn()?;

        let mut state = self.inner.lock().unwrap();
        let id = state.next_id;
        state.next_id += 1;
        state.children.insert(id, child);
        Ok(ProcessHandle { id })
    }

    pub fn stop(&self, handle: &ProcessHandle) -> CoreResult<()> {
        let mut state = self.inner.lock().unwrap();
        if let Some(mut child) = state.children.remove(&handle.id) {
            let _ = child.kill();
        }
        Ok(())
    }

    pub fn is_running(&self, handle: &ProcessHandle) -> bool {
        let mut state = self.inner.lock().unwrap();
        if let Some(child) = state.children.get_mut(&handle.id) {
            match child.try_wait() {
                Ok(Some(_)) => false,
                Ok(None) => true,
                Err(_) => false,
            }
        } else {
            false
        }
    }
}

pub struct ProjectorLauncher {
    manager: ProcessManager,
}

impl ProjectorLauncher {
    pub fn new(manager: ProcessManager) -> Self {
        Self { manager }
    }

    pub fn launch(&self, cfg: &CoreConfig) -> CoreResult<ProcessHandle> {
        self.manager.launch_projector(cfg)
    }
}
