use crate::app::AppError;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub keba_ip: String,
    pub keba_udp_port: u16,
    pub poll_interval_ms: u64,
    pub db_path: String,
    pub http_bind: String,
    pub debounce_samples: usize,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, AppError> {
        Self::from_lookup(|key| std::env::var(key).ok())
    }

    fn from_lookup<F>(lookup: F) -> Result<Self, AppError>
    where
        F: Fn(&str) -> Option<String>,
    {
        let keba_ip = lookup("KEBA_IP")
            .map(|v| v.trim().to_string())
            .filter(|v| !v.is_empty())
            .ok_or_else(|| AppError::config("KEBA_IP is required"))?;

        Ok(Self {
            keba_ip,
            keba_udp_port: parse_or_default(&lookup, "KEBA_UDP_PORT", 7090_u16)?,
            poll_interval_ms: parse_or_default(&lookup, "POLL_INTERVAL_MS", 1000_u64)?,
            db_path: lookup("DB_PATH")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "/var/lib/keba/keba.db".to_string()),
            http_bind: lookup("HTTP_BIND")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "0.0.0.0:8080".to_string()),
            debounce_samples: parse_or_default(&lookup, "DEBOUNCE_SAMPLES", 2_usize)?,
        })
    }
}

fn parse_or_default<T, F>(lookup: &F, key: &str, default: T) -> Result<T, AppError>
where
    T: std::str::FromStr + Copy,
    F: Fn(&str) -> Option<String>,
{
    match lookup(key) {
        Some(raw) => raw
            .trim()
            .parse::<T>()
            .map_err(|_| AppError::config(format!("{key} must be a valid number"))),
        None => Ok(default),
    }
}

#[cfg(test)]
mod tests {
    use super::AppConfig;

    #[test]
    fn rejects_missing_keba_ip() {
        let result = AppConfig::from_lookup(|_| None);
        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid configuration: KEBA_IP is required"
        );
    }

    #[test]
    fn applies_defaults_for_optional_fields() {
        let result = AppConfig::from_lookup(|key| match key {
            "KEBA_IP" => Some("192.168.1.10".to_string()),
            _ => None,
        })
        .expect("config should be valid");

        assert_eq!(result.keba_ip, "192.168.1.10");
        assert_eq!(result.keba_udp_port, 7090);
        assert_eq!(result.poll_interval_ms, 1000);
        assert_eq!(result.db_path, "/var/lib/keba/keba.db");
        assert_eq!(result.http_bind, "0.0.0.0:8080");
        assert_eq!(result.debounce_samples, 2);
    }

    #[test]
    fn rejects_invalid_numeric_values() {
        let result = AppConfig::from_lookup(|key| match key {
            "KEBA_IP" => Some("192.168.1.10".to_string()),
            "POLL_INTERVAL_MS" => Some("abc".to_string()),
            _ => None,
        });

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid configuration: POLL_INTERVAL_MS must be a valid number"
        );
    }
}
