use actix_web::{HttpRequest, HttpResponse, Responder, get, web};
use chrono::{NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::keba_udp::KebaUdpClient;
const API_KEY_HEADER: &str = "X-API-Key";
const API_KEY_VALUE: &str = "1r0m";

#[derive(Clone)]
pub struct ApiState {
    pub report100_stations: Vec<Report100Station>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report100Station {
    pub logical_name: String,
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Serialize, PartialEq)]
pub struct LatestStationSessionResponse {
    #[serde(rename = "reportId")]
    pub report_id: u16,
    #[serde(rename = "kWh")]
    pub kwh: f64,
    pub started: Option<i64>,
    pub ended: Option<i64>,
    #[serde(rename = "CardId")]
    pub card_id: Value,
}

#[derive(Debug, Deserialize)]
pub struct Report100Query {
    pub station: Option<String>,
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(health)
        .service(get_carport_latest_report100_endpoint)
        .service(get_entrance_latest_report100_endpoint);
}

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({ "status": "ok" }))
}

#[get("/sessions/carport/latest")]
async fn get_carport_latest_report100_endpoint(
    req: HttpRequest,
    state: web::Data<ApiState>,
) -> impl Responder {
    if !has_valid_api_key(&req) {
        return unauthorized_response();
    }
    latest_report100_response(&state, "carport")
}

#[get("/sessions/entrance/latest")]
async fn get_entrance_latest_report100_endpoint(
    req: HttpRequest,
    state: web::Data<ApiState>,
) -> impl Responder {
    if !has_valid_api_key(&req) {
        return unauthorized_response();
    }
    latest_report100_response(&state, "entrance")
}

fn has_valid_api_key(req: &HttpRequest) -> bool {
    req.headers()
        .get(API_KEY_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(|value| value == API_KEY_VALUE)
        .unwrap_or(false)
}

fn unauthorized_response() -> HttpResponse {
    HttpResponse::Unauthorized().json(serde_json::json!({
        "error": "missing or invalid API key"
    }))
}

fn latest_report100_response(state: &ApiState, station_name: &str) -> HttpResponse {
    const REPORT_SEARCH_START: u16 = 100;
    const REPORT_SEARCH_END: u16 = 130;

    let Some(station) = state
        .report100_stations
        .iter()
        .find(|entry| entry.logical_name == station_name)
    else {
        return HttpResponse::NotFound().json(serde_json::json!({
            "error": format!("station mapping for '{station_name}' is not configured")
        }));
    };

    let client = match KebaUdpClient::new(&station.ip, station.port) {
        Ok(client) => client,
        Err(error) => {
            return HttpResponse::BadGateway().json(serde_json::json!({
                "error": format!("failed to initialize report 100 client: {error}")
            }));
        }
    };

    let now_ms = Utc::now().timestamp_millis();
    let mut last_fetch_error: Option<String> = None;

    for report_id in REPORT_SEARCH_START..=REPORT_SEARCH_END {
        let payload = match client.get_report(report_id) {
            Ok(payload) => payload,
            Err(error) => {
                last_fetch_error = Some(format!("report {report_id}: {error}"));
                continue;
            }
        };

        let Some(object) = payload.as_object() else {
            last_fetch_error = Some(format!("report {report_id}: payload must be a json object"));
            continue;
        };

        let view = extract_latest_station_session_view(object, now_ms);
        if has_complete_session_data(&view) {
            return HttpResponse::Ok().json(LatestStationSessionResponse {
                report_id,
                kwh: view.kwh.unwrap_or(0.0),
                started: view.started,
                ended: view.ended,
                card_id: view.card_id,
            });
        }
    }

    let mut error_message =
        "reports 100-130 do not contain started/end timestamps and E Pres > 0".to_string();
    if let Some(fetch_error) = last_fetch_error {
        error_message.push_str(&format!(" (last error: {fetch_error})"));
    }

    HttpResponse::BadGateway().json(serde_json::json!({
        "error": error_message
    }))
}

struct LatestStationSessionView {
    kwh: Option<f64>,
    started: Option<i64>,
    ended: Option<i64>,
    card_id: Value,
}

fn has_complete_session_data(view: &LatestStationSessionView) -> bool {
    matches!(view.started, Some(started) if started > 0)
        && matches!(view.ended, Some(ended) if ended > 0)
        && matches!(view.kwh, Some(kwh) if kwh > 0.0)
}

fn extract_latest_station_session_view(
    object: &serde_json::Map<String, Value>,
    now_ms: i64,
) -> LatestStationSessionView {
    let sec_from_report = find_number_from_object(object, &["Sec", "sec", "Seconds", "seconds"]);
    let kwh = find_number_from_object(
        object,
        &[
            "E Pres",
            "E pres",
            "Energy Session",
            "energy_present_session",
        ],
    )
    .map(normalize_session_kwh);
    let started = find_value_from_object(
        object,
        &[
            "started[s]",
            "Started[s]",
            "started",
            "Started",
            "start",
            "session_start",
            "Session Start",
        ],
    )
    .and_then(|value| parse_session_timestamp_ms(value, now_ms, sec_from_report));
    let ended = find_value_from_object(
        object,
        &[
            "ended[s]",
            "Ended[s]",
            "ended",
            "Ended",
            "end",
            "session_end",
            "Session End",
        ],
    )
    .and_then(|value| parse_session_timestamp_ms(value, now_ms, sec_from_report));
    let card_id = find_value_from_object(
        object,
        &[
            "RFID", "RFID tag", "RFID Tag", "CardId", "Card ID", "card_id",
        ],
    )
    .cloned()
    .unwrap_or(Value::Null);

    LatestStationSessionView {
        kwh,
        started,
        ended,
        card_id,
    }
}

fn normalize_session_kwh(raw_energy: f64) -> f64 {
    if raw_energy >= 1000.0 {
        raw_energy / 10_000.0
    } else {
        raw_energy
    }
}

fn find_value_from_object<'a>(
    object: &'a serde_json::Map<String, Value>,
    aliases: &[&str],
) -> Option<&'a Value> {
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

fn find_number_from_object(
    object: &serde_json::Map<String, Value>,
    aliases: &[&str],
) -> Option<f64> {
    find_value_from_object(object, aliases).and_then(parse_number)
}

fn parse_number(value: &Value) -> Option<f64> {
    match value {
        Value::Number(number) => number.as_f64(),
        Value::String(text) => text.trim().replace(',', ".").parse::<f64>().ok(),
        _ => None,
    }
}

fn parse_absolute_timestamp_ms(value: &Value) -> Option<i64> {
    let Value::String(text) = value else {
        return None;
    };
    if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(text) {
        return Some(parsed.timestamp_millis());
    }
    if let Ok(parsed) = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.3f") {
        return Some(Utc.from_utc_datetime(&parsed).timestamp_millis());
    }
    None
}

fn parse_session_timestamp_ms(
    value: &Value,
    now_ms: i64,
    sec_from_report100: Option<f64>,
) -> Option<i64> {
    if let Some(absolute_ms) = parse_absolute_timestamp_ms(value)
        && is_plausible_absolute_timestamp_ms(absolute_ms, now_ms)
    {
        return Some(absolute_ms);
    }
    if let Some(raw_numeric) = parse_number(value)
        && let Some(numeric_timestamp_ms) = parse_numeric_timestamp_ms(raw_numeric, now_ms)
    {
        return Some(numeric_timestamp_ms);
    }

    if let Some(sec_now) = sec_from_report100
        && let Some(raw_seconds) = parse_number(value)
        && (0.0..1_000_000_000_000.0).contains(&raw_seconds)
    {
        let ts = (now_ms as f64) - ((sec_now - raw_seconds) * 1000.0);
        let ts_ms = ts.round() as i64;
        if is_plausible_absolute_timestamp_ms(ts_ms, now_ms) {
            return Some(ts_ms);
        }
    }
    None
}

fn is_plausible_absolute_timestamp_ms(timestamp_ms: i64, now_ms: i64) -> bool {
    const MIN_PLAUSIBLE_TIMESTAMP_MS: i64 = 946_684_800_000; // 2000-01-01T00:00:00Z
    const MAX_FUTURE_DRIFT_MS: i64 = 86_400_000; // +24h
    timestamp_ms >= MIN_PLAUSIBLE_TIMESTAMP_MS && timestamp_ms <= now_ms + MAX_FUTURE_DRIFT_MS
}

fn parse_numeric_timestamp_ms(raw_value: f64, now_ms: i64) -> Option<i64> {
    if !raw_value.is_finite() || raw_value < 0.0 {
        return None;
    }
    const MIN_PLAUSIBLE_TIMESTAMP_SECONDS: f64 = 946_684_800.0; // 2000-01-01T00:00:00Z
    const MAX_FUTURE_DRIFT_SECONDS: f64 = 86_400.0; // +24h
    let now_seconds = (now_ms as f64) / 1000.0;
    if (MIN_PLAUSIBLE_TIMESTAMP_SECONDS..=now_seconds + MAX_FUTURE_DRIFT_SECONDS)
        .contains(&raw_value)
    {
        return Some((raw_value * 1000.0).round() as i64);
    }
    let raw_ms = raw_value.round() as i64;
    if is_plausible_absolute_timestamp_ms(raw_ms, now_ms) {
        return Some(raw_ms);
    }
    None
}

fn normalize_key(input: &str) -> String {
    input
        .chars()
        .filter(|char| char.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect()
}

#[cfg(test)]
mod tests {
    use std::net::UdpSocket;
    use std::sync::{Arc, Mutex};
    use std::thread;
    use std::time::Duration;

    use actix_web::{App, body::to_bytes, http::StatusCode, test, web};
    use chrono::Utc;

    use super::{ApiState, Report100Station, configure_routes, parse_absolute_timestamp_ms};

    fn empty_state() -> ApiState {
        ApiState {
            report100_stations: Vec::new(),
        }
    }

    #[actix_web::test]
    async fn health_endpoint_returns_ok() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(empty_state()))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn sessions_endpoint_returns_unauthorized_without_api_key() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(empty_state()))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/carport/latest")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(json["error"], "missing or invalid API key");
    }

    #[actix_web::test]
    async fn carport_latest_endpoint_returns_report100_payload() {
        let responder = UdpSocket::bind("127.0.0.1:0").expect("responder socket should bind");
        responder
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout should be configurable");
        let responder_port = responder
            .local_addr()
            .expect("addr should be available")
            .port();

        let responder_handle = thread::spawn(move || {
            let mut buffer = [0_u8; 512];
            loop {
                let (size, from) = match responder.recv_from(&mut buffer) {
                    Ok(tuple) => tuple,
                    Err(_) => break,
                };
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                if cmd == "report 100" {
                    let payload = r#"{"E Pres":12.34,"started":"2026-03-01 16:40:19.000","ended":"2026-03-02 04:01:59.000","RFID tag":"ABC123"}"#;
                    responder
                        .send_to(payload.as_bytes(), from)
                        .expect("responder send should succeed");
                }
            }
        });

        let mut state = empty_state();
        state.report100_stations = vec![Report100Station {
            logical_name: "carport".to_string(),
            ip: "127.0.0.1".to_string(),
            port: responder_port,
        }];
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/carport/latest")
            .insert_header(("X-API-Key", "1r0m"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        let expected_started =
            parse_absolute_timestamp_ms(&serde_json::json!("2026-03-01 16:40:19.000"))
                .expect("timestamp should parse");
        let expected_ended =
            parse_absolute_timestamp_ms(&serde_json::json!("2026-03-02 04:01:59.000"))
                .expect("timestamp should parse");
        assert_eq!(json["kWh"], 12.34);
        assert_eq!(json["reportId"], 100);
        assert_eq!(json["started"], expected_started);
        assert_eq!(json["ended"], expected_ended);
        assert_eq!(json["CardId"], "ABC123");

        let shutdown_socket = UdpSocket::bind("127.0.0.1:0").expect("shutdown socket should bind");
        shutdown_socket
            .send_to(
                b"shutdown-test-responder",
                format!("127.0.0.1:{responder_port}"),
            )
            .expect("shutdown message should be sent");
        responder_handle
            .join()
            .expect("responder thread should terminate cleanly");
    }

    #[actix_web::test]
    async fn entrance_latest_endpoint_returns_404_when_station_mapping_missing() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(empty_state()))
                .configure(configure_routes),
        )
        .await;
        let req = test::TestRequest::get()
            .uri("/sessions/entrance/latest")
            .insert_header(("X-API-Key", "1r0m"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn carport_latest_endpoint_falls_back_to_report101_when_ended_is_zero() {
        let responder = UdpSocket::bind("127.0.0.1:0").expect("responder socket should bind");
        responder
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout should be configurable");
        let responder_port = responder
            .local_addr()
            .expect("addr should be available")
            .port();

        let responder_handle = thread::spawn(move || {
            let mut buffer = [0_u8; 512];
            loop {
                let (size, from) = match responder.recv_from(&mut buffer) {
                    Ok(tuple) => tuple,
                    Err(_) => break,
                };
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                let payload = match cmd.as_str() {
                    "report 100" => {
                        r#"{"E Pres":9.87,"started":"2026-03-01 16:40:19.000","ended":0,"RFID tag":"ABC123"}"#
                    }
                    "report 101" => {
                        r#"{"E Pres":7.65,"started":"2026-03-01 16:40:19.000","ended":"2026-03-02 04:01:59.000","RFID tag":"XYZ999"}"#
                    }
                    _ => continue,
                };
                responder
                    .send_to(payload.as_bytes(), from)
                    .expect("responder send should succeed");
            }
        });

        let mut state = empty_state();
        state.report100_stations = vec![Report100Station {
            logical_name: "carport".to_string(),
            ip: "127.0.0.1".to_string(),
            port: responder_port,
        }];
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/carport/latest")
            .insert_header(("X-API-Key", "1r0m"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        let expected_started =
            parse_absolute_timestamp_ms(&serde_json::json!("2026-03-01 16:40:19.000"))
                .expect("timestamp should parse");
        let expected_ended =
            parse_absolute_timestamp_ms(&serde_json::json!("2026-03-02 04:01:59.000"))
                .expect("timestamp should parse");
        assert_eq!(json["kWh"], 7.65);
        assert_eq!(json["reportId"], 101);
        assert_eq!(json["started"], expected_started);
        assert_eq!(json["ended"], expected_ended);
        assert_eq!(json["CardId"], "XYZ999");

        let shutdown_socket = UdpSocket::bind("127.0.0.1:0").expect("shutdown socket should bind");
        shutdown_socket
            .send_to(
                b"shutdown-test-responder",
                format!("127.0.0.1:{responder_port}"),
            )
            .expect("shutdown message should be sent");
        responder_handle
            .join()
            .expect("responder thread should terminate cleanly");
    }

    #[actix_web::test]
    async fn carport_latest_endpoint_checks_reports_until_valid_payload_is_found() {
        let seen_commands = Arc::new(Mutex::new(Vec::<String>::new()));
        let seen_commands_responder = Arc::clone(&seen_commands);

        let responder = UdpSocket::bind("127.0.0.1:0").expect("responder socket should bind");
        responder
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout should be configurable");
        let responder_port = responder
            .local_addr()
            .expect("addr should be available")
            .port();

        let responder_handle = thread::spawn(move || {
            let mut buffer = [0_u8; 512];
            loop {
                let (size, from) = match responder.recv_from(&mut buffer) {
                    Ok(tuple) => tuple,
                    Err(_) => break,
                };
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }

                seen_commands_responder
                    .lock()
                    .expect("command log mutex should be lockable")
                    .push(cmd.clone());

                let payload = match cmd.as_str() {
                    "report 100" => {
                        r#"{"E Pres":4.2,"started":"2026-03-01 16:40:19.000","ended":0,"RFID tag":"R100"}"#
                    }
                    "report 101" => {
                        r#"{"E Pres":0,"started":"2026-03-01 16:40:19.000","ended":"2026-03-02 04:01:59.000","RFID tag":"R101"}"#
                    }
                    "report 102" => {
                        r#"{"E Pres":1.23,"started":null,"ended":"2026-03-02 04:01:59.000","RFID tag":"R102"}"#
                    }
                    "report 103" => {
                        r#"{"E Pres":6.54,"started":"2026-03-01 16:40:19.000","ended":"2026-03-02 04:01:59.000","RFID tag":"R103"}"#
                    }
                    _ => continue,
                };

                responder
                    .send_to(payload.as_bytes(), from)
                    .expect("responder send should succeed");
            }
        });

        let mut state = empty_state();
        state.report100_stations = vec![Report100Station {
            logical_name: "carport".to_string(),
            ip: "127.0.0.1".to_string(),
            port: responder_port,
        }];
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/carport/latest")
            .insert_header(("X-API-Key", "1r0m"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        let expected_started =
            parse_absolute_timestamp_ms(&serde_json::json!("2026-03-01 16:40:19.000"))
                .expect("timestamp should parse");
        let expected_ended =
            parse_absolute_timestamp_ms(&serde_json::json!("2026-03-02 04:01:59.000"))
                .expect("timestamp should parse");
        assert_eq!(json["kWh"], 6.54);
        assert_eq!(json["reportId"], 103);
        assert_eq!(json["started"], expected_started);
        assert_eq!(json["ended"], expected_ended);
        assert_eq!(json["CardId"], "R103");

        let commands = seen_commands
            .lock()
            .expect("command log mutex should be lockable")
            .clone();
        assert_eq!(
            commands,
            vec!["report 100", "report 101", "report 102", "report 103"]
        );

        let shutdown_socket = UdpSocket::bind("127.0.0.1:0").expect("shutdown socket should bind");
        shutdown_socket
            .send_to(
                b"shutdown-test-responder",
                format!("127.0.0.1:{responder_port}"),
            )
            .expect("shutdown message should be sent");
        responder_handle
            .join()
            .expect("responder thread should terminate cleanly");
    }

    #[actix_web::test]
    async fn carport_latest_endpoint_converts_relative_started_ended_with_sec() {
        let responder = UdpSocket::bind("127.0.0.1:0").expect("responder socket should bind");
        responder
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout should be configurable");
        let responder_port = responder
            .local_addr()
            .expect("addr should be available")
            .port();

        let responder_handle = thread::spawn(move || {
            let mut buffer = [0_u8; 512];
            loop {
                let (size, from) = match responder.recv_from(&mut buffer) {
                    Ok(tuple) => tuple,
                    Err(_) => break,
                };
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                if cmd == "report 100" {
                    let payload = r#"{"E Pres":4.2,"Sec":"40000000","started":"26332000","ended":"35131000","RFID tag":"REL1"}"#;
                    responder
                        .send_to(payload.as_bytes(), from)
                        .expect("responder send should succeed");
                }
            }
        });

        let mut state = empty_state();
        state.report100_stations = vec![Report100Station {
            logical_name: "carport".to_string(),
            ip: "127.0.0.1".to_string(),
            port: responder_port,
        }];
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let now_before = Utc::now().timestamp_millis();
        let req = test::TestRequest::get()
            .uri("/sessions/carport/latest")
            .insert_header(("X-API-Key", "1r0m"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let now_after = Utc::now().timestamp_millis();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");

        let sec_now = 40_000_000.0_f64;
        let started_raw = 26_332_000.0_f64;
        let ended_raw = 35_131_000.0_f64;
        let expected_started_min =
            (now_before as f64 - (sec_now - started_raw) * 1000.0) as i64 - 2000;
        let expected_started_max =
            (now_after as f64 - (sec_now - started_raw) * 1000.0) as i64 + 2000;
        let expected_ended_min = (now_before as f64 - (sec_now - ended_raw) * 1000.0) as i64 - 2000;
        let expected_ended_max = (now_after as f64 - (sec_now - ended_raw) * 1000.0) as i64 + 2000;

        let started = json["started"].as_i64().expect("started should be i64");
        let ended = json["ended"].as_i64().expect("ended should be i64");
        assert!(started >= expected_started_min && started <= expected_started_max);
        assert!(ended >= expected_ended_min && ended <= expected_ended_max);
        assert_eq!(json["kWh"], 4.2);
        assert_eq!(json["reportId"], 100);
        assert_eq!(json["CardId"], "REL1");

        let shutdown_socket = UdpSocket::bind("127.0.0.1:0").expect("shutdown socket should bind");
        shutdown_socket
            .send_to(
                b"shutdown-test-responder",
                format!("127.0.0.1:{responder_port}"),
            )
            .expect("shutdown message should be sent");
        responder_handle
            .join()
            .expect("responder thread should terminate cleanly");
    }

    #[actix_web::test]
    async fn carport_latest_endpoint_uses_sec_fallback_when_absolute_numeric_timestamp_is_implausible(
    ) {
        let responder = UdpSocket::bind("127.0.0.1:0").expect("responder socket should bind");
        responder
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout should be configurable");
        let responder_port = responder
            .local_addr()
            .expect("addr should be available")
            .port();

        let responder_handle = thread::spawn(move || {
            let mut buffer = [0_u8; 512];
            loop {
                let (size, from) = match responder.recv_from(&mut buffer) {
                    Ok(tuple) => tuple,
                    Err(_) => break,
                };
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                if cmd == "report 100" {
                    let payload =
                        r#"{"E Pres":4.2,"Sec":"40000000","started":26332000,"ended":35131000,"RFID tag":"REL2"}"#;
                    responder
                        .send_to(payload.as_bytes(), from)
                        .expect("responder send should succeed");
                }
            }
        });

        let mut state = empty_state();
        state.report100_stations = vec![Report100Station {
            logical_name: "carport".to_string(),
            ip: "127.0.0.1".to_string(),
            port: responder_port,
        }];
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let now_before = Utc::now().timestamp_millis();
        let req = test::TestRequest::get()
            .uri("/sessions/carport/latest")
            .insert_header(("X-API-Key", "1r0m"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let now_after = Utc::now().timestamp_millis();
        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");

        let sec_now = 40_000_000.0_f64;
        let started_raw = 26_332_000.0_f64;
        let ended_raw = 35_131_000.0_f64;
        let expected_started_min =
            (now_before as f64 - (sec_now - started_raw) * 1000.0) as i64 - 2000;
        let expected_started_max =
            (now_after as f64 - (sec_now - started_raw) * 1000.0) as i64 + 2000;
        let expected_ended_min =
            (now_before as f64 - (sec_now - ended_raw) * 1000.0) as i64 - 2000;
        let expected_ended_max =
            (now_after as f64 - (sec_now - ended_raw) * 1000.0) as i64 + 2000;

        let started = json["started"].as_i64().expect("started should be i64");
        let ended = json["ended"].as_i64().expect("ended should be i64");
        assert!(started >= expected_started_min && started <= expected_started_max);
        assert!(ended >= expected_ended_min && ended <= expected_ended_max);
        assert_eq!(json["kWh"], 4.2);
        assert_eq!(json["reportId"], 100);
        assert_eq!(json["CardId"], "REL2");

        let shutdown_socket = UdpSocket::bind("127.0.0.1:0").expect("shutdown socket should bind");
        shutdown_socket
            .send_to(
                b"shutdown-test-responder",
                format!("127.0.0.1:{responder_port}"),
            )
            .expect("shutdown message should be sent");
        responder_handle
            .join()
            .expect("responder thread should terminate cleanly");
    }

    #[actix_web::test]
    async fn carport_latest_endpoint_converts_e_pres_wh_to_kwh() {
        let responder = UdpSocket::bind("127.0.0.1:0").expect("responder socket should bind");
        responder
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout should be configurable");
        let responder_port = responder
            .local_addr()
            .expect("addr should be available")
            .port();

        let responder_handle = thread::spawn(move || {
            let mut buffer = [0_u8; 512];
            loop {
                let (size, from) = match responder.recv_from(&mut buffer) {
                    Ok(tuple) => tuple,
                    Err(_) => break,
                };
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                if cmd == "report 100" {
                    let payload = r#"{"E Pres":81984,"started":"2026-03-01 16:40:19.000","ended":"2026-03-02 04:01:59.000","RFID tag":"WH1"}"#;
                    responder
                        .send_to(payload.as_bytes(), from)
                        .expect("responder send should succeed");
                }
            }
        });

        let mut state = empty_state();
        state.report100_stations = vec![Report100Station {
            logical_name: "carport".to_string(),
            ip: "127.0.0.1".to_string(),
            port: responder_port,
        }];
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/carport/latest")
            .insert_header(("X-API-Key", "1r0m"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(json["kWh"], 8.1984);
        assert_eq!(json["reportId"], 100);
        assert_eq!(json["CardId"], "WH1");

        let shutdown_socket = UdpSocket::bind("127.0.0.1:0").expect("shutdown socket should bind");
        shutdown_socket
            .send_to(
                b"shutdown-test-responder",
                format!("127.0.0.1:{responder_port}"),
            )
            .expect("shutdown message should be sent");
        responder_handle
            .join()
            .expect("responder thread should terminate cleanly");
    }

    #[actix_web::test]
    async fn carport_latest_endpoint_prefers_started_ended_seconds_fields() {
        let responder = UdpSocket::bind("127.0.0.1:0").expect("responder socket should bind");
        responder
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout should be configurable");
        let responder_port = responder
            .local_addr()
            .expect("addr should be available")
            .port();

        let responder_handle = thread::spawn(move || {
            let mut buffer = [0_u8; 512];
            loop {
                let (size, from) = match responder.recv_from(&mut buffer) {
                    Ok(tuple) => tuple,
                    Err(_) => break,
                };
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                if cmd == "report 100" {
                    let payload = r#"{"E Pres":20064,"Sec":187494,"started[s]":182170,"ended[s]":184901,"started":"182170000","ended":"184901000","RFID tag":"SSEC1"}"#;
                    responder
                        .send_to(payload.as_bytes(), from)
                        .expect("responder send should succeed");
                }
            }
        });

        let mut state = empty_state();
        state.report100_stations = vec![Report100Station {
            logical_name: "carport".to_string(),
            ip: "127.0.0.1".to_string(),
            port: responder_port,
        }];
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let now_before = Utc::now().timestamp_millis();
        let req = test::TestRequest::get()
            .uri("/sessions/carport/latest")
            .insert_header(("X-API-Key", "1r0m"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        let now_after = Utc::now().timestamp_millis();
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");

        let sec_now = 187_494.0_f64;
        let started_raw = 182_170.0_f64;
        let ended_raw = 184_901.0_f64;
        let expected_started_min =
            (now_before as f64 - (sec_now - started_raw) * 1000.0) as i64 - 2000;
        let expected_started_max =
            (now_after as f64 - (sec_now - started_raw) * 1000.0) as i64 + 2000;
        let expected_ended_min =
            (now_before as f64 - (sec_now - ended_raw) * 1000.0) as i64 - 2000;
        let expected_ended_max =
            (now_after as f64 - (sec_now - ended_raw) * 1000.0) as i64 + 2000;

        let started = json["started"].as_i64().expect("started should be i64");
        let ended = json["ended"].as_i64().expect("ended should be i64");
        assert!(started >= expected_started_min && started <= expected_started_max);
        assert!(ended >= expected_ended_min && ended <= expected_ended_max);
        assert_eq!(json["kWh"], 2.0064);
        assert_eq!(json["reportId"], 100);
        assert_eq!(json["CardId"], "SSEC1");

        let shutdown_socket = UdpSocket::bind("127.0.0.1:0").expect("shutdown socket should bind");
        shutdown_socket
            .send_to(
                b"shutdown-test-responder",
                format!("127.0.0.1:{responder_port}"),
            )
            .expect("shutdown message should be sent");
        responder_handle
            .join()
            .expect("responder thread should terminate cleanly");
    }

    #[actix_web::test]
    async fn carport_latest_endpoint_accepts_epoch_seconds_from_started_seconds_field() {
        let responder = UdpSocket::bind("127.0.0.1:0").expect("responder socket should bind");
        responder
            .set_read_timeout(Some(Duration::from_secs(1)))
            .expect("read timeout should be configurable");
        let responder_port = responder
            .local_addr()
            .expect("addr should be available")
            .port();

        let responder_handle = thread::spawn(move || {
            let mut buffer = [0_u8; 512];
            loop {
                let (size, from) = match responder.recv_from(&mut buffer) {
                    Ok(tuple) => tuple,
                    Err(_) => break,
                };
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                if cmd == "report 100" {
                    let payload = r#"{"E Pres":81984,"Sec":779184,"started[s]":1772546661,"ended[s]":1772605949,"started":"2026-03-03 14:04:21.000","ended":"2026-03-04 06:32:29.000","RFID tag":"SSEC2"}"#;
                    responder
                        .send_to(payload.as_bytes(), from)
                        .expect("responder send should succeed");
                }
            }
        });

        let mut state = empty_state();
        state.report100_stations = vec![Report100Station {
            logical_name: "carport".to_string(),
            ip: "127.0.0.1".to_string(),
            port: responder_port,
        }];
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/carport/latest")
            .insert_header(("X-API-Key", "1r0m"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(json["started"], 1_772_546_661_000_i64);
        assert_eq!(json["ended"], 1_772_605_949_000_i64);
        assert_eq!(json["kWh"], 8.1984);
        assert_eq!(json["reportId"], 100);
        assert_eq!(json["CardId"], "SSEC2");

        let shutdown_socket = UdpSocket::bind("127.0.0.1:0").expect("shutdown socket should bind");
        shutdown_socket
            .send_to(
                b"shutdown-test-responder",
                format!("127.0.0.1:{responder_port}"),
            )
            .expect("shutdown message should be sent");
        responder_handle
            .join()
            .expect("responder thread should terminate cleanly");
    }
}
