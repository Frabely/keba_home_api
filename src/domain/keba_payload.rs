use serde_json::{Map, Value};
use thiserror::Error;

#[derive(Debug, Clone, PartialEq)]
pub struct Report2 {
    pub plugged: bool,
    pub seconds: Option<u64>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Report3 {
    pub present_session_kwh: Option<f64>,
    pub total_kwh: Option<f64>,
}

#[derive(Debug, Error, PartialEq)]
pub enum ParseError {
    #[error("payload must be a JSON object")]
    InvalidPayloadType,
    #[error("missing required field: {0}")]
    MissingField(&'static str),
}

#[derive(Clone, Copy)]
enum EnergyUnit {
    Wh,
    KWh,
}

#[derive(Clone, Copy)]
struct EnergyAlias {
    key: &'static str,
    unit: EnergyUnit,
}

const PLUG_KEYS: &[&str] = &["Plug", "plug", "plugged"];
const STATE_KEYS: &[&str] = &["State", "state", "Charging state", "charging_state"];
const SECONDS_KEYS: &[&str] = &["Seconds", "seconds", "Sec", "sec", "plugged seconds"];

const PRESENT_ENERGY_KEYS: &[EnergyAlias] = &[
    EnergyAlias {
        key: "E pres",
        unit: EnergyUnit::Wh,
    },
    EnergyAlias {
        key: "Energy (present session)",
        unit: EnergyUnit::KWh,
    },
    EnergyAlias {
        key: "energy_present_session",
        unit: EnergyUnit::KWh,
    },
    EnergyAlias {
        key: "EnergyPresentSession",
        unit: EnergyUnit::KWh,
    },
];

const TOTAL_ENERGY_KEYS: &[EnergyAlias] = &[
    EnergyAlias {
        key: "Total energy",
        unit: EnergyUnit::Wh,
    },
    EnergyAlias {
        key: "Energy (total)",
        unit: EnergyUnit::KWh,
    },
    EnergyAlias {
        key: "energy_total",
        unit: EnergyUnit::KWh,
    },
    EnergyAlias {
        key: "EnergyTotal",
        unit: EnergyUnit::KWh,
    },
];

pub fn parse_report2(payload: &Value) -> Result<Report2, ParseError> {
    let object = payload.as_object().ok_or(ParseError::InvalidPayloadType)?;

    let plugged = find_number(object, PLUG_KEYS)
        .map(|value| value > 0.0)
        .or_else(|| find_number(object, STATE_KEYS).map(|value| value > 0.0))
        .ok_or(ParseError::MissingField("Plug|State"))?;

    let seconds = find_number(object, SECONDS_KEYS).and_then(f64_to_non_negative_u64);

    Ok(Report2 { plugged, seconds })
}

pub fn parse_report3(payload: &Value) -> Result<Report3, ParseError> {
    let object = payload.as_object().ok_or(ParseError::InvalidPayloadType)?;

    let present_session_kwh = find_energy_kwh(object, PRESENT_ENERGY_KEYS);
    let total_kwh = find_energy_kwh(object, TOTAL_ENERGY_KEYS);

    if present_session_kwh.is_none() && total_kwh.is_none() {
        return Err(ParseError::MissingField(
            "E pres|Energy (present session)|Total energy",
        ));
    }

    Ok(Report3 {
        present_session_kwh,
        total_kwh,
    })
}

fn find_energy_kwh(object: &Map<String, Value>, aliases: &[EnergyAlias]) -> Option<f64> {
    aliases.iter().find_map(|alias| {
        find_value(object, &[alias.key]).and_then(|value| {
            let number = parse_f64(value)?;
            Some(match alias.unit {
                EnergyUnit::Wh => number / 1000.0,
                EnergyUnit::KWh => number,
            })
        })
    })
}

fn find_number(object: &Map<String, Value>, aliases: &[&str]) -> Option<f64> {
    find_value(object, aliases).and_then(parse_f64)
}

fn find_value<'a>(object: &'a Map<String, Value>, aliases: &[&str]) -> Option<&'a Value> {
    for alias in aliases {
        if let Some(value) = object.get(*alias) {
            return Some(value);
        }
    }

    let normalized_aliases: Vec<String> =
        aliases.iter().map(|alias| normalize_key(alias)).collect();

    object.iter().find_map(|(key, value)| {
        let normalized_key = normalize_key(key);
        if normalized_aliases
            .iter()
            .any(|alias| alias == &normalized_key)
        {
            Some(value)
        } else {
            None
        }
    })
}

fn normalize_key(value: &str) -> String {
    value
        .chars()
        .filter(|char| char.is_ascii_alphanumeric())
        .flat_map(|char| char.to_lowercase())
        .collect()
}

fn parse_f64(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => parse_f64_from_text(text),
        _ => None,
    }
}

fn parse_f64_from_text(text: &str) -> Option<f64> {
    extract_numeric_tokens(text).into_iter().find_map(|token| {
        normalize_numeric_token(&token).and_then(|normalized| normalized.parse::<f64>().ok())
    })
}

fn extract_numeric_tokens(text: &str) -> Vec<String> {
    let mut tokens = Vec::new();
    let mut current = String::new();

    for char in text.chars() {
        if char.is_ascii_digit() || char == ',' || char == '.' || char == '-' {
            current.push(char);
        } else if !current.is_empty() {
            tokens.push(std::mem::take(&mut current));
        }
    }

    if !current.is_empty() {
        tokens.push(current);
    }

    tokens
}

fn normalize_numeric_token(token: &str) -> Option<String> {
    let comma_count = token.matches(',').count();
    let dot_count = token.matches('.').count();

    if comma_count > 0 && dot_count > 0 {
        let comma_index = token.rfind(',')?;
        let dot_index = token.rfind('.')?;
        if comma_index > dot_index {
            return Some(token.replace('.', "").replace(',', "."));
        }
        return Some(token.replace(',', ""));
    }

    if comma_count > 0 {
        return Some(token.replace(',', "."));
    }

    if dot_count > 1 {
        return Some(token.replace('.', ""));
    }

    Some(token.to_string())
}

fn f64_to_non_negative_u64(value: f64) -> Option<u64> {
    if !value.is_finite() || value < 0.0 {
        return None;
    }

    Some(value.floor() as u64)
}

#[cfg(test)]
mod tests {
    use super::{ParseError, Report2, Report3, parse_report2, parse_report3};
    use serde_json::json;

    #[test]
    fn parses_report2_from_plug_field() {
        let payload = json!({"Plug": 7, "Seconds": 4264958});

        let parsed = parse_report2(&payload).expect("report2 must parse");

        assert_eq!(
            parsed,
            Report2 {
                plugged: true,
                seconds: Some(4_264_958),
            }
        );
    }

    #[test]
    fn parses_report2_with_state_fallback_and_string_seconds() {
        let payload = json!({"state": "1", "seconds": "4264958"});

        let parsed = parse_report2(&payload).expect("report2 must parse");

        assert_eq!(
            parsed,
            Report2 {
                plugged: true,
                seconds: Some(4_264_958),
            }
        );
    }

    #[test]
    fn parses_report2_with_normalized_aliases() {
        let payload = json!({"Charging state": 0, "Plugged Seconds": 99});

        let parsed = parse_report2(&payload).expect("report2 must parse");

        assert_eq!(
            parsed,
            Report2 {
                plugged: false,
                seconds: Some(99),
            }
        );
    }

    #[test]
    fn report2_requires_plug_or_state() {
        let payload = json!({"Seconds": 100});

        let parsed = parse_report2(&payload);

        assert_eq!(parsed, Err(ParseError::MissingField("Plug|State")));
    }

    #[test]
    fn parses_report3_wh_to_kwh_for_e_pres() {
        let payload = json!({"E pres": 10830, "Total energy": 28193080});

        let parsed = parse_report3(&payload).expect("report3 must parse");

        assert_eq!(
            parsed,
            Report3 {
                present_session_kwh: Some(10.83),
                total_kwh: Some(28_193.08),
            }
        );
    }

    #[test]
    fn parses_report3_kwh_string_with_comma() {
        let payload = json!({"Energy (present session)": "10,83 kWh"});

        let parsed = parse_report3(&payload).expect("report3 must parse");

        assert_eq!(
            parsed,
            Report3 {
                present_session_kwh: Some(10.83),
                total_kwh: None,
            }
        );
    }

    #[test]
    fn parses_report3_accepts_total_only() {
        let payload = json!({"Energy (total)": "28193.08"});

        let parsed = parse_report3(&payload).expect("report3 must parse");

        assert_eq!(
            parsed,
            Report3 {
                present_session_kwh: None,
                total_kwh: Some(28_193.08),
            }
        );
    }

    #[test]
    fn report3_requires_any_energy_field() {
        let payload = json!({"report": 3});

        let parsed = parse_report3(&payload);

        assert_eq!(
            parsed,
            Err(ParseError::MissingField(
                "E pres|Energy (present session)|Total energy"
            ))
        );
    }

    #[test]
    fn rejects_non_object_payload() {
        let payload = json!([1, 2, 3]);

        let parsed = parse_report2(&payload);

        assert_eq!(parsed, Err(ParseError::InvalidPayloadType));
    }
}
