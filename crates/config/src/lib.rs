mod config_struct;
mod load;
mod secrets;

pub use config_struct::{Config, ConsoleConfig};
pub use load::load_config;
