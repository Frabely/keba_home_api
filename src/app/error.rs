use thiserror::Error;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("failed to initialize logging: {0}")]
    LoggingInit(String),
    #[error("invalid configuration: {0}")]
    Config(String),
}

impl AppError {
    pub fn logging_init<E: std::fmt::Display>(error: E) -> Self {
        Self::LoggingInit(error.to_string())
    }

    pub fn config<E: std::fmt::Display>(error: E) -> Self {
        Self::Config(error.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::AppError;

    #[test]
    fn maps_logging_init_error_message() {
        let err = AppError::logging_init("subscriber already set");
        assert_eq!(
            err.to_string(),
            "failed to initialize logging: subscriber already set"
        );
    }
}
