//! Layered configuration: defaults → user → project → env → CLI flags.

pub mod loader;
pub mod manager;
pub mod schema;

pub use loader::{load_config, project_config_path, user_config_path, CliFlags};
pub use manager::ConfigManager;
pub use schema::CliConfig;
