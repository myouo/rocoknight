pub mod config;
pub mod error;
pub mod logging;
pub mod process;
pub mod update;
pub mod cache;
pub mod window;
pub mod window_embed;

pub use config::{CoreConfig, GameConfig, LauncherConfig};
pub use error::{CoreError, CoreResult};
pub use process::{ProcessHandle, ProcessManager, ProjectorLauncher};
pub use window_embed::{EmbedRect, RawHwnd};
