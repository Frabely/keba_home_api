use std::collections::HashMap;

use reqwest::{Client, Url};
use serde_json::Value;
use thiserror::Error;

const ENGINE_START_COUNT_KEY: &str = "Hka_Bd.ulAnzahlStarts";
const ENGINE_OPERATING_HOURS_KEY: &str = "Hka_Bd.ulBetriebssekunden";
const INTERNAL_ELECTRICITY_KEY: &str = "Hka_Bd.ulArbeitElektr";
const HEAT_OUTPUT_KEY: &str = "Hka_Bd.ulArbeitThermHka";
const MAINTENANCE_BASELINE_HOURS_KEY: &str = "Wartung_Cache.ulBetriebssekundenBei";
const BUDERUS_START_COUNT_KEY: &str = "Brenner_Bd.ulAnzahlStarts";
const BUDERUS_OPERATING_HOURS_KEY: &str = "Brenner_Bd.ulBetriebssekunden";
const MAINTENANCE_INTERVAL_HOURS: f64 = 3_500.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DachsRequestProfile {
    F233,
    F235,
}

#[derive(Debug, Clone, PartialEq)]
pub struct DachsStatus {
    pub starts: u64,
    pub bh: f64,
    pub electricity_internal: f64,
    pub heat: f64,
    pub maintenance: f64,
    pub buderus_starts: Option<u64>,
    pub buderus_bh: Option<f64>,
}

#[derive(Debug, Error)]
pub enum DachsHttpError {
    #[error("invalid dachs base url: {0}")]
    InvalidBaseUrl(String),
    #[error("dachs upstream request failed: {0}")]
    Transport(String),
    #[error("dachs upstream returned HTTP {status}: {body}")]
    UpstreamStatus { status: u16, body: String },
    #[error("dachs payload is invalid: {0}")]
    InvalidPayload(String),
}

pub async fn fetch_dachs_status(
    client: &Client,
    base_url: &str,
    username: Option<&str>,
    password: Option<&str>,
    profile: DachsRequestProfile,
) -> Result<DachsStatus, DachsHttpError> {
    let url = build_dachs_status_url(base_url, profile)?;
    let mut request = client.get(url);
    let username = username
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .unwrap_or("");
    let password = password.map(str::trim).filter(|value| !value.is_empty());
    if !username.is_empty() || password.is_some() {
        request = request.basic_auth(username, password);
    }
    let response = request
        .send()
        .await
        .map_err(|error| DachsHttpError::Transport(error.to_string()))?;
    let status = response.status();
    let body = response
        .text()
        .await
        .map_err(|error| DachsHttpError::Transport(error.to_string()))?;

    if !status.is_success() {
        return Err(DachsHttpError::UpstreamStatus {
            status: status.as_u16(),
            body: truncate_body(&body),
        });
    }

    parse_dachs_status_payload(&body, profile)
}

fn build_dachs_status_url(
    base_url: &str,
    profile: DachsRequestProfile,
) -> Result<Url, DachsHttpError> {
    let mut url = Url::parse(base_url)
        .map_err(|error| DachsHttpError::InvalidBaseUrl(error.to_string()))?
        .join("getKey")
        .map_err(|error| DachsHttpError::InvalidBaseUrl(error.to_string()))?;

    {
        let mut query = url.query_pairs_mut();
        for key in request_keys(profile) {
            query.append_pair("k", key);
        }
    }

    Ok(url)
}

fn parse_dachs_status_payload(
    body: &str,
    profile: DachsRequestProfile,
) -> Result<DachsStatus, DachsHttpError> {
    let values = parse_payload_map(body)?;

    let starts = parse_u64_value(&values, ENGINE_START_COUNT_KEY)?;
    let bh = parse_f64_value(&values, ENGINE_OPERATING_HOURS_KEY)?;
    let electricity_internal = parse_f64_value(&values, INTERNAL_ELECTRICITY_KEY)?;
    let heat = parse_f64_value(&values, HEAT_OUTPUT_KEY)?;
    let maintenance_at = parse_f64_value(&values, MAINTENANCE_BASELINE_HOURS_KEY)?;

    let (buderus_starts, buderus_bh) = match profile {
        DachsRequestProfile::F233 => (
            Some(parse_u64_value(&values, BUDERUS_START_COUNT_KEY)?),
            Some(parse_f64_value(&values, BUDERUS_OPERATING_HOURS_KEY)?),
        ),
        DachsRequestProfile::F235 => (None, None),
    };

    Ok(DachsStatus {
        starts,
        bh,
        electricity_internal,
        heat,
        maintenance: round_to_three_decimals(MAINTENANCE_INTERVAL_HOURS - (bh - maintenance_at)),
        buderus_starts,
        buderus_bh,
    })
}

fn request_keys(profile: DachsRequestProfile) -> &'static [&'static str] {
    match profile {
        DachsRequestProfile::F233 => &[
            ENGINE_START_COUNT_KEY,
            ENGINE_OPERATING_HOURS_KEY,
            INTERNAL_ELECTRICITY_KEY,
            HEAT_OUTPUT_KEY,
            MAINTENANCE_BASELINE_HOURS_KEY,
            BUDERUS_START_COUNT_KEY,
            BUDERUS_OPERATING_HOURS_KEY,
        ],
        DachsRequestProfile::F235 => &[
            ENGINE_START_COUNT_KEY,
            ENGINE_OPERATING_HOURS_KEY,
            INTERNAL_ELECTRICITY_KEY,
            HEAT_OUTPUT_KEY,
            MAINTENANCE_BASELINE_HOURS_KEY,
        ],
    }
}

fn parse_payload_map(body: &str) -> Result<HashMap<String, Value>, DachsHttpError> {
    if let Ok(value) = serde_json::from_str::<Value>(body) {
        return value_to_map(&value);
    }

    parse_text_map(body)
}

fn value_to_map(value: &Value) -> Result<HashMap<String, Value>, DachsHttpError> {
    match value {
        Value::Object(object) => Ok(object
            .iter()
            .map(|(key, value)| (key.clone(), value.clone()))
            .collect()),
        Value::Array(items) => {
            let mut values = HashMap::new();
            for item in items {
                match item {
                    Value::Object(object) if object.len() == 1 => {
                        let (key, value) = object
                            .iter()
                            .next()
                            .expect("single-entry object should contain one pair");
                        values.insert(key.clone(), value.clone());
                    }
                    Value::Object(object) => {
                        let key = find_object_string_field(object, &["key", "name", "k"])
                            .ok_or_else(|| {
                                DachsHttpError::InvalidPayload(
                                    "array entry is missing a key/name/k field".to_string(),
                                )
                            })?;
                        let value = find_object_value_field(object, &["value", "val", "v"])
                            .ok_or_else(|| {
                                DachsHttpError::InvalidPayload(format!(
                                    "array entry for '{key}' is missing a value field"
                                ))
                            })?;
                        values.insert(key, value.clone());
                    }
                    _ => {
                        return Err(DachsHttpError::InvalidPayload(
                            "json array entries must be objects".to_string(),
                        ));
                    }
                }
            }
            Ok(values)
        }
        _ => Err(DachsHttpError::InvalidPayload(
            "json payload must be an object or array".to_string(),
        )),
    }
}

fn parse_text_map(body: &str) -> Result<HashMap<String, Value>, DachsHttpError> {
    let mut values = HashMap::new();

    for line in body.lines().map(str::trim).filter(|line| !line.is_empty()) {
        let Some((key, value)) = line.split_once('=').or_else(|| line.split_once(':')) else {
            return Err(DachsHttpError::InvalidPayload(format!(
                "cannot parse line '{line}'"
            )));
        };

        values.insert(
            key.trim().to_string(),
            Value::String(value.trim().trim_matches('"').to_string()),
        );
    }

    if values.is_empty() {
        return Err(DachsHttpError::InvalidPayload(
            "payload is empty".to_string(),
        ));
    }

    Ok(values)
}

fn parse_f64_value(
    values: &HashMap<String, Value>,
    key: &'static str,
) -> Result<f64, DachsHttpError> {
    let value = values
        .get(key)
        .ok_or_else(|| DachsHttpError::InvalidPayload(format!("missing key '{key}'")))?;
    parse_numeric_value(value, key)
}

fn parse_u64_value(
    values: &HashMap<String, Value>,
    key: &'static str,
) -> Result<u64, DachsHttpError> {
    let parsed = parse_f64_value(values, key)?;
    if !parsed.is_finite() || parsed < 0.0 {
        return Err(DachsHttpError::InvalidPayload(format!(
            "key '{key}' must contain a non-negative integer"
        )));
    }

    let rounded = parsed.round();
    if (parsed - rounded).abs() > f64::EPSILON {
        return Err(DachsHttpError::InvalidPayload(format!(
            "key '{key}' must contain an integer value"
        )));
    }

    Ok(rounded as u64)
}

fn parse_numeric_value(value: &Value, key: &'static str) -> Result<f64, DachsHttpError> {
    match value {
        Value::Number(number) => number.as_f64().ok_or_else(|| {
            DachsHttpError::InvalidPayload(format!("key '{key}' contains a non-f64 number"))
        }),
        Value::String(text) => parse_numeric_text(text).map_err(|error| {
            DachsHttpError::InvalidPayload(format!("key '{key}' contains '{text}': {error}"))
        }),
        Value::Bool(boolean) => Err(DachsHttpError::InvalidPayload(format!(
            "key '{key}' contains unsupported bool value {boolean}"
        ))),
        Value::Null => Err(DachsHttpError::InvalidPayload(format!(
            "key '{key}' contains null"
        ))),
        _ => Err(DachsHttpError::InvalidPayload(format!(
            "key '{key}' contains unsupported nested json"
        ))),
    }
}

fn parse_numeric_text(text: &str) -> Result<f64, &'static str> {
    let normalized = normalize_numeric_text(text);
    normalized.parse::<f64>().map_err(|_| "not a valid number")
}

fn normalize_numeric_text(text: &str) -> String {
    let trimmed = text.trim().replace(' ', "");
    if trimmed.contains(',') {
        return trimmed.replace('.', "").replace(',', ".");
    }

    if trimmed.matches('.').count() > 1 {
        return trimmed.replace('.', "");
    }

    trimmed
}

fn find_object_string_field(
    object: &serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<String> {
    find_object_value_field(object, keys).and_then(|value| match value {
        Value::String(text) => Some(text.clone()),
        _ => None,
    })
}

fn find_object_value_field<'a>(
    object: &'a serde_json::Map<String, Value>,
    keys: &[&str],
) -> Option<&'a Value> {
    keys.iter().find_map(|key| object.get(*key))
}

fn truncate_body(body: &str) -> String {
    const MAX_LEN: usize = 200;
    if body.len() <= MAX_LEN {
        return body.to_string();
    }

    format!("{}...", &body[..MAX_LEN])
}

fn round_to_three_decimals(value: f64) -> f64 {
    (value * 1000.0).round() / 1000.0
}

#[cfg(test)]
mod tests {
    use super::{
        DachsRequestProfile, DachsStatus, build_dachs_status_url, parse_dachs_status_payload,
    };

    #[test]
    fn parses_json_object_with_german_decimal_numbers() {
        let payload = r#"{
            "Hka_Bd.ulAnzahlStarts": "476",
            "Hka_Bd.ulBetriebssekunden": "47.088,020",
            "Hka_Bd.ulArbeitElektr": "12.345,678",
            "Hka_Bd.ulArbeitThermHka": "98.765,432",
            "Wartung_Cache.ulBetriebssekundenBei": "46.687,516",
            "Brenner_Bd.ulAnzahlStarts": "91",
            "Brenner_Bd.ulBetriebssekunden": "2.468,100"
        }"#;

        let parsed = parse_dachs_status_payload(payload, DachsRequestProfile::F233)
            .expect("payload should parse");

        assert_eq!(
            parsed,
            DachsStatus {
                starts: 476,
                bh: 47_088.020,
                electricity_internal: 12_345.678,
                heat: 98_765.432,
                maintenance: 3_099.496,
                buderus_starts: Some(91),
                buderus_bh: Some(2_468.100),
            }
        );
    }

    #[test]
    fn parses_key_value_text_payload() {
        let payload = "\
Hka_Bd.ulAnzahlStarts=10\n\
Hka_Bd.ulBetriebssekunden=1000\n\
Hka_Bd.ulArbeitElektr=123.4\n\
Hka_Bd.ulArbeitThermHka=567.8\n\
Wartung_Cache.ulBetriebssekundenBei=600\n\
Brenner_Bd.ulAnzahlStarts=11\n\
Brenner_Bd.ulBetriebssekunden=22\n";

        let parsed = parse_dachs_status_payload(payload, DachsRequestProfile::F233)
            .expect("payload should parse");

        assert_eq!(parsed.starts, 10);
        assert_eq!(parsed.bh, 1000.0);
        assert_eq!(parsed.electricity_internal, 123.4);
        assert_eq!(parsed.heat, 567.8);
        assert_eq!(parsed.maintenance, 3100.0);
        assert_eq!(parsed.buderus_starts, Some(11));
        assert_eq!(parsed.buderus_bh, Some(22.0));
    }

    #[test]
    fn builds_upstream_url_with_request_keys_for_f233() {
        let url = build_dachs_status_url("http://192.168.233.91:8080", DachsRequestProfile::F233)
            .expect("url should build");

        assert_eq!(
            url.as_str(),
            "http://192.168.233.91:8080/getKey?k=Hka_Bd.ulAnzahlStarts&k=Hka_Bd.ulBetriebssekunden&k=Hka_Bd.ulArbeitElektr&k=Hka_Bd.ulArbeitThermHka&k=Wartung_Cache.ulBetriebssekundenBei&k=Brenner_Bd.ulAnzahlStarts&k=Brenner_Bd.ulBetriebssekunden"
        );
    }

    #[test]
    fn builds_upstream_url_without_buderus_keys_for_f235() {
        let url = build_dachs_status_url("http://192.168.233.91:8080", DachsRequestProfile::F235)
            .expect("url should build");

        assert_eq!(
            url.as_str(),
            "http://192.168.233.91:8080/getKey?k=Hka_Bd.ulAnzahlStarts&k=Hka_Bd.ulBetriebssekunden&k=Hka_Bd.ulArbeitElektr&k=Hka_Bd.ulArbeitThermHka&k=Wartung_Cache.ulBetriebssekundenBei"
        );
    }
}
