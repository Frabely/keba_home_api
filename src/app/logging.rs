use tracing_subscriber::{EnvFilter, fmt};

use crate::app::AppError;

pub fn init() -> Result<(), AppError> {
    let filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    let format = LogFormat::from_env();

    match format {
        LogFormat::Compact => fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_file(false)
            .with_line_number(false)
            .compact()
            .try_init()
            .map_err(AppError::logging_init),
        LogFormat::Pretty => fmt()
            .with_env_filter(filter)
            .with_target(false)
            .with_thread_ids(false)
            .with_thread_names(false)
            .with_file(false)
            .with_line_number(false)
            .pretty()
            .try_init()
            .map_err(AppError::logging_init),
        LogFormat::Full => fmt()
            .with_env_filter(filter)
            .with_target(true)
            .try_init()
            .map_err(AppError::logging_init),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LogFormat {
    Compact,
    Pretty,
    Full,
}

impl LogFormat {
    fn from_env() -> Self {
        match std::env::var("LOG_FORMAT")
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref()
        {
            Some("pretty") => Self::Pretty,
            Some("full") => Self::Full,
            _ => Self::Compact,
        }
    }
}
