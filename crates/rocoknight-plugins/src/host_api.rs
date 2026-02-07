use crate::permissions::PermissionSet;
use rocoknight_core::{CoreConfig, ProcessHandle};

pub trait HostApi: Send + Sync {
    fn permissions(&self) -> PermissionSet;

    fn get_config(&self) -> CoreConfig;
    fn set_config(&self, cfg: CoreConfig);

    fn launch(&self) -> anyhow::Result<ProcessHandle>;
    fn restart(&self, handle: ProcessHandle) -> anyhow::Result<ProcessHandle>;
    fn stop(&self, handle: ProcessHandle) -> anyhow::Result<()>;

    fn notify(&self, title: &str, body: &str);
}

