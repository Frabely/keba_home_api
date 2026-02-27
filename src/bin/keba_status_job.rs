use std::net::UdpSocket;
use std::thread;
use std::time::Duration;

use chrono::Utc;
use serde_json::{Map, Value};

const POLL_INTERVAL: Duration = Duration::from_secs(5);
const UDP_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy)]
struct Station {
    name: &'static str,
    ip: &'static str,
    port: u16,
}

const STATIONS: &[Station] = &[
    Station {
        name: "KEBA Carport",
        ip: "192.168.233.98",
        port: 7090,
    },
    Station {
        name: "KEBA Eingang",
        ip: "192.168.233.91",
        port: 7090,
    },
];

#[derive(Debug, Clone)]
struct StationStatus {
    plugged: bool,
    enabled: bool,
    fault: bool,
    charging: bool,
    state: Option<i64>,
    error1: i64,
    error2: i64,
    max_current: Option<f64>,
    power_w: Option<f64>,
    session_kwh: Option<f64>,
    total_kwh: Option<f64>,
    status_text: &'static str,
}

fn main() {
    println!(
        "Starte KEBA Status-Job (Intervall: {}s) fuer {} Stationen...",
        POLL_INTERVAL.as_secs(),
        STATIONS.len()
    );

    let socket = match UdpSocket::bind("0.0.0.0:7090") {
        Ok(socket) => socket,
        Err(err) => {
            println!(
                "[{}] FEHLER: UDP-Port 7090 lokal konnte nicht gebunden werden: {}",
                now_iso(),
                err
            );
            return;
        }
    };

    if let Err(err) = socket.set_read_timeout(Some(UDP_TIMEOUT)) {
        println!(
            "[{}] FEHLER: Konnte UDP Read-Timeout nicht setzen: {}",
            now_iso(),
            err
        );
        return;
    }
    if let Err(err) = socket.set_write_timeout(Some(UDP_TIMEOUT)) {
        println!(
            "[{}] FEHLER: Konnte UDP Write-Timeout nicht setzen: {}",
            now_iso(),
            err
        );
        return;
    }

    loop {
        for station in STATIONS {
            poll_station(&socket, station);
        }
        println!();
        thread::sleep(POLL_INTERVAL);
    }
}

fn poll_station(socket: &UdpSocket, station: &Station) {
    let report2 = match send_report(socket, station, 2) {
        Ok(value) => value,
        Err(err) => {
            println!(
                "[{}] {} ({}): FEHLER report 2: {}",
                now_iso(),
                station.name,
                station.ip,
                err
            );
            return;
        }
    };

    let report3 = match send_report(socket, station, 3) {
        Ok(value) => value,
        Err(err) => {
            println!(
                "[{}] {} ({}): FEHLER report 3: {}",
                now_iso(),
                station.name,
                station.ip,
                err
            );
            return;
        }
    };

    let status = build_status(&report2, &report3);

    println!(
        "[{}] {} ({}) | Status: {}",
        now_iso(),
        station.name,
        station.ip,
        status.status_text
    );
    println!(
        "  Stecker: {} | Laden: {} | Freigegeben: {} | Fehler: {}",
        yes_no(status.plugged),
        yes_no(status.charging),
        yes_no(status.enabled),
        if status.fault {
            format!("ja (Error1={}, Error2={})", status.error1, status.error2)
        } else {
            "nein".to_string()
        }
    );

    match status.state {
        Some(state) => println!("  State: {}", state),
        None => println!("  State: n/a"),
    }

    match status.max_current {
        Some(max_current) => println!("  Max curr: {:.0} mA", max_current),
        None => println!("  Max curr: n/a"),
    }

    match status.power_w {
        Some(power_w) => println!("  Leistung: {:.0} W", power_w),
        None => println!("  Leistung: n/a"),
    }

    match status.session_kwh {
        Some(kwh) => println!(
            "  Aktuell geladene Session-Energie (E pres): {:.3} kWh",
            kwh
        ),
        None => println!("  Aktuell geladene Session-Energie (E pres): n/a"),
    }

    match status.total_kwh {
        Some(kwh) => println!("  Gesamtzaehler (E total): {:.3} kWh", kwh),
        None => println!("  Gesamtzaehler (E total): n/a"),
    }
}

fn send_report(socket: &UdpSocket, station: &Station, report_id: u8) -> Result<Value, String> {
    let command = format!("report {report_id}");
    socket
        .send_to(
            command.as_bytes(),
            format!("{}:{}", station.ip, station.port),
        )
        .map_err(|err| err.to_string())?;

    let mut buffer = [0_u8; 4096];
    let (size, from) = socket
        .recv_from(&mut buffer)
        .map_err(|err| err.to_string())?;

    if from.ip().to_string() != station.ip {
        return Err(format!(
            "unerwartete Antwort von {from}; erwartet wurde {}:{}",
            station.ip, station.port
        ));
    }

    serde_json::from_slice(&buffer[..size]).map_err(|err| err.to_string())
}

fn now_iso() -> String {
    Utc::now().to_rfc3339()
}

fn yes_no(value: bool) -> &'static str {
    if value { "ja" } else { "nein" }
}

fn build_status(report2: &Value, report3: &Value) -> StationStatus {
    let report2_obj = report2.as_object();
    let report3_obj = report3.as_object();

    let plug = report2_obj
        .and_then(|obj| find_number(obj, &["Plug"]))
        .unwrap_or(0.0);
    let plugged = plug != 0.0;

    let enable_sys = report2_obj
        .and_then(|obj| find_number(obj, &["Enable sys"]))
        .unwrap_or(0.0);
    let enable_user = report2_obj
        .and_then(|obj| find_number(obj, &["Enable user"]))
        .unwrap_or(0.0);
    let max_current = report2_obj.and_then(|obj| find_number(obj, &["Max curr"]));
    let enabled = enable_sys == 1.0 && enable_user == 1.0 && max_current.unwrap_or(0.0) > 0.0;

    let error1 = report2_obj
        .and_then(|obj| find_number(obj, &["Error1"]))
        .unwrap_or(0.0) as i64;
    let error2 = report2_obj
        .and_then(|obj| find_number(obj, &["Error2"]))
        .unwrap_or(0.0) as i64;
    let fault = error1 != 0 || error2 != 0;

    let power_w = report3_obj.and_then(|obj| find_number(obj, &["P"]));
    let charging = power_w.unwrap_or(0.0) > 0.0;

    let session_kwh = report3_obj.and_then(find_session_kwh);
    let total_kwh = report3_obj.and_then(find_total_kwh);

    let state = report2_obj
        .and_then(|obj| find_number(obj, &["State"]))
        .map(|value| value as i64);

    let status_text = if fault {
        "Fehler"
    } else if !plugged {
        "Nicht angesteckt"
    } else if charging {
        "Lädt"
    } else if !enabled {
        "Angesteckt, aber gesperrt/deaktiviert"
    } else {
        "Angesteckt, wartet/bereit"
    };

    StationStatus {
        plugged,
        enabled,
        fault,
        charging,
        state,
        error1,
        error2,
        max_current,
        power_w,
        session_kwh,
        total_kwh,
        status_text,
    }
}

fn find_session_kwh(obj: &Map<String, Value>) -> Option<f64> {
    if let Some(raw) = find_number(obj, &["E pres"]) {
        return Some(raw / 10_000.0);
    }
    find_number(obj, &["Energy (present session)", "energy_present_session"])
}

fn find_total_kwh(obj: &Map<String, Value>) -> Option<f64> {
    if let Some(raw) = find_number(obj, &["E total", "Total energy"]) {
        return Some(raw / 10_000.0);
    }
    find_number(obj, &["Energy (total)", "energy_total"])
}

fn find_number(obj: &Map<String, Value>, aliases: &[&str]) -> Option<f64> {
    for alias in aliases {
        if let Some(value) = obj.get(*alias)
            && let Some(parsed) = parse_number(value)
        {
            return Some(parsed);
        }
    }

    let normalized_aliases: Vec<String> =
        aliases.iter().map(|alias| normalize_key(alias)).collect();

    obj.iter().find_map(|(key, value)| {
        let normalized_key = normalize_key(key);
        if normalized_aliases
            .iter()
            .any(|alias| alias == &normalized_key)
        {
            parse_number(value)
        } else {
            None
        }
    })
}

fn normalize_key(input: &str) -> String {
    input
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

fn parse_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => parse_number_from_text(text),
        _ => None,
    }
}

fn parse_number_from_text(text: &str) -> Option<f64> {
    let cleaned = text.trim().replace(',', ".");
    let token = cleaned
        .split(|c: char| !c.is_ascii_digit() && c != '.' && c != '-')
        .find(|part| !part.is_empty())?;
    token.parse::<f64>().ok()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::{build_status, find_session_kwh, find_total_kwh};

    #[test]
    fn derives_waiting_status_when_plugged_enabled_and_not_charging() {
        let report2 = json!({
            "Plug": 7,
            "Enable sys": 1,
            "Enable user": 1,
            "Max curr": 32000,
            "Error1": 0,
            "Error2": 0,
            "State": 2
        });
        let report3 = json!({
            "P": 0,
            "E pres": 41210,
            "E total": 283467494
        });

        let status = build_status(&report2, &report3);

        assert_eq!(status.status_text, "Angesteckt, wartet/bereit");
        assert_eq!(status.charging, false);
        assert_eq!(status.session_kwh, Some(4.121));
        assert_eq!(status.total_kwh, Some(28_346.7494));
    }

    #[test]
    fn derives_disabled_status_when_not_enabled() {
        let report2 = json!({
            "Plug": 7,
            "Enable sys": 0,
            "Enable user": 0,
            "Max curr": 0,
            "Error1": 0,
            "Error2": 0,
            "State": 5
        });
        let report3 = json!({"P": 0});

        let status = build_status(&report2, &report3);

        assert_eq!(status.status_text, "Angesteckt, aber gesperrt/deaktiviert");
        assert_eq!(status.enabled, false);
    }

    #[test]
    fn parses_energy_from_explicit_kwh_fields() {
        let report3 = json!({
            "Energy (present session)": 3.5,
            "Energy (total)": 28000.5
        });
        let obj = report3.as_object().expect("json object expected");

        assert_eq!(find_session_kwh(obj), Some(3.5));
        assert_eq!(find_total_kwh(obj), Some(28_000.5));
    }
}
