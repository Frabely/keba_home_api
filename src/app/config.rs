use crate::app::AppError;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub keba_ip: String,
    pub keba_udp_port: u16,
    pub keba_source: KebaSource,
    pub keba_modbus_port: u16,
    pub keba_modbus_unit_id: u8,
    pub keba_modbus_energy_factor_wh: f64,
    pub keba_debug_data_file: Option<String>,
    pub results_output_file: Option<String>,
    pub poll_interval_ms: u64,
    pub db_path: String,
    pub http_bind: String,
    pub debounce_samples: usize,
    pub station_id: Option<String>,
    pub status_log_interval_seconds: u64,
    pub status_stations: Vec<StatusStationConfig>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KebaSource {
    Udp,
    Modbus,
    DebugFile,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StatusStationConfig {
    pub name: String,
    pub ip: String,
    pub port: u16,
}

impl AppConfig {
    pub fn from_env() -> Result<Self, AppError> {
        let _ = dotenvy::dotenv();
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

        let config = Self {
            keba_ip,
            keba_udp_port: parse_or_default(&lookup, "KEBA_UDP_PORT", 7090_u16)?,
            keba_source: parse_keba_source(&lookup)?,
            keba_modbus_port: parse_or_default(&lookup, "KEBA_MODBUS_PORT", 502_u16)?,
            keba_modbus_unit_id: parse_or_default(&lookup, "KEBA_MODBUS_UNIT_ID", 255_u8)?,
            keba_modbus_energy_factor_wh: parse_or_default(
                &lookup,
                "KEBA_MODBUS_ENERGY_FACTOR_WH",
                0.1_f64,
            )?,
            keba_debug_data_file: lookup("KEBA_DEBUG_DATA_FILE")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            results_output_file: lookup("RESULTS_OUTPUT_FILE")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            poll_interval_ms: parse_or_default(&lookup, "POLL_INTERVAL_MS", 1000_u64)?,
            db_path: lookup("DB_PATH")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(default_db_path),
            http_bind: lookup("HTTP_BIND")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty())
                .unwrap_or_else(|| "0.0.0.0:8080".to_string()),
            debounce_samples: parse_or_default(&lookup, "DEBOUNCE_SAMPLES", 2_usize)?,
            station_id: lookup("STATION_ID")
                .map(|v| v.trim().to_string())
                .filter(|v| !v.is_empty()),
            status_log_interval_seconds: parse_or_default(
                &lookup,
                "STATUS_LOG_INTERVAL_SECONDS",
                5_u64,
            )?,
            status_stations: parse_status_stations(&lookup)?,
        };

        if config.keba_source == KebaSource::DebugFile && config.keba_debug_data_file.is_none() {
            return Err(AppError::config(
                "KEBA_DEBUG_DATA_FILE is required when KEBA_SOURCE=debug_file",
            ));
        }

        Ok(config)
    }
}

fn parse_keba_source<F>(lookup: &F) -> Result<KebaSource, AppError>
where
    F: Fn(&str) -> Option<String>,
{
    match lookup("KEBA_SOURCE")
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("udp")
        .to_ascii_lowercase()
        .as_str()
    {
        "udp" => Ok(KebaSource::Udp),
        "modbus" => Ok(KebaSource::Modbus),
        "debug_file" => Ok(KebaSource::DebugFile),
        _ => Err(AppError::config(
            "KEBA_SOURCE must be one of: udp, modbus, debug_file",
        )),
    }
}

fn default_db_path() -> String {
    if cfg!(windows) {
        ".\\data\\keba.db".to_string()
    } else {
        "/var/lib/keba/keba.db".to_string()
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

fn parse_status_stations<F>(lookup: &F) -> Result<Vec<StatusStationConfig>, AppError>
where
    F: Fn(&str) -> Option<String>,
{
    let raw = lookup("STATUS_STATIONS").unwrap_or_else(|| {
        "KEBA Carport@192.168.233.98:7090;KEBA Eingang@192.168.233.91:7090".to_string()
    });

    let mut stations = Vec::new();

    for entry in raw
        .split(';')
        .map(str::trim)
        .filter(|part| !part.is_empty())
    {
        let (name_raw, endpoint_raw) = entry.split_once('@').ok_or_else(|| {
            AppError::config(format!(
                "STATUS_STATIONS entry must look like Name@IP:Port: {entry}"
            ))
        })?;

        let (ip_raw, port_raw) = endpoint_raw.rsplit_once(':').ok_or_else(|| {
            AppError::config(format!(
                "STATUS_STATIONS endpoint must look like IP:Port: {endpoint_raw}"
            ))
        })?;

        let name = name_raw.trim();
        let ip = ip_raw.trim();
        let port = port_raw.trim().parse::<u16>().map_err(|_| {
            AppError::config(format!("STATUS_STATIONS has invalid port: {port_raw}"))
        })?;

        if name.is_empty() {
            return Err(AppError::config(
                "STATUS_STATIONS entry has empty station name",
            ));
        }
        if ip.is_empty() {
            return Err(AppError::config(
                "STATUS_STATIONS entry has empty station ip",
            ));
        }

        stations.push(StatusStationConfig {
            name: name.to_string(),
            ip: ip.to_string(),
            port,
        });
    }

    if stations.is_empty() {
        return Err(AppError::config(
            "STATUS_STATIONS must contain at least one station",
        ));
    }

    Ok(stations)
}

#[cfg(test)]
mod tests {
    use super::{AppConfig, KebaSource, StatusStationConfig};

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
        assert_eq!(result.keba_source, KebaSource::Udp);
        assert_eq!(result.keba_modbus_port, 502);
        assert_eq!(result.keba_modbus_unit_id, 255);
        assert!((result.keba_modbus_energy_factor_wh - 0.1).abs() < f64::EPSILON);
        assert_eq!(result.keba_debug_data_file, None);
        assert_eq!(result.results_output_file, None);
        assert_eq!(result.poll_interval_ms, 1000);
        if cfg!(windows) {
            assert_eq!(result.db_path, ".\\data\\keba.db");
        } else {
            assert_eq!(result.db_path, "/var/lib/keba/keba.db");
        }
        assert_eq!(result.http_bind, "0.0.0.0:8080");
        assert_eq!(result.debounce_samples, 2);
        assert_eq!(result.station_id, None);
        assert_eq!(result.status_log_interval_seconds, 5);
        assert_eq!(
            result.status_stations,
            vec![
                StatusStationConfig {
                    name: "KEBA Carport".to_string(),
                    ip: "192.168.233.98".to_string(),
                    port: 7090,
                },
                StatusStationConfig {
                    name: "KEBA Eingang".to_string(),
                    ip: "192.168.233.91".to_string(),
                    port: 7090,
                },
            ]
        );
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

    #[test]
    fn rejects_invalid_keba_source() {
        let result = AppConfig::from_lookup(|key| match key {
            "KEBA_IP" => Some("192.168.1.10".to_string()),
            "KEBA_SOURCE" => Some("serial".to_string()),
            _ => None,
        });

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid configuration: KEBA_SOURCE must be one of: udp, modbus, debug_file"
        );
    }

    #[test]
    fn requires_debug_data_file_for_debug_source() {
        let result = AppConfig::from_lookup(|key| match key {
            "KEBA_IP" => Some("192.168.1.10".to_string()),
            "KEBA_SOURCE" => Some("debug_file".to_string()),
            _ => None,
        });

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid configuration: KEBA_DEBUG_DATA_FILE is required when KEBA_SOURCE=debug_file"
        );
    }

    #[test]
    fn parses_custom_status_stations() {
        let result = AppConfig::from_lookup(|key| match key {
            "KEBA_IP" => Some("192.168.1.10".to_string()),
            "STATUS_STATIONS" => {
                Some("Carport@192.168.1.101:7090;Eingang@192.168.1.102:7091".to_string())
            }
            "STATUS_LOG_INTERVAL_SECONDS" => Some("10".to_string()),
            _ => None,
        })
        .expect("config should be valid");

        assert_eq!(result.status_log_interval_seconds, 10);
        assert_eq!(result.status_stations.len(), 2);
        assert_eq!(result.status_stations[0].name, "Carport");
        assert_eq!(result.status_stations[0].ip, "192.168.1.101");
        assert_eq!(result.status_stations[0].port, 7090);
        assert_eq!(result.status_stations[1].name, "Eingang");
        assert_eq!(result.status_stations[1].ip, "192.168.1.102");
        assert_eq!(result.status_stations[1].port, 7091);
    }

    #[test]
    fn rejects_invalid_status_stations_format() {
        let result = AppConfig::from_lookup(|key| match key {
            "KEBA_IP" => Some("192.168.1.10".to_string()),
            "STATUS_STATIONS" => Some("invalid-format".to_string()),
            _ => None,
        });

        assert!(result.is_err());
        assert_eq!(
            result.unwrap_err().to_string(),
            "invalid configuration: STATUS_STATIONS entry must look like Name@IP:Port: invalid-format"
        );
    }
}
