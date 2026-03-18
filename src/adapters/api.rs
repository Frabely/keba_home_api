use actix_web::{
    Error, HttpResponse, Responder, body::EitherBody, body::MessageBody, dev::ServiceRequest,
    dev::ServiceResponse, http::header, middleware::Next, middleware::from_fn, web,
};
use chrono::{NaiveDateTime, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::keba_udp::KebaUdpClient;
use crate::app::services::{SessionQueryHandler, SqliteSessionService};

const API_V1_PREFIX: &str = "/api/v1";

#[derive(Clone)]
pub struct ApiState {
    pub report100_stations: Vec<Report100Station>,
    pub session_query_service: Option<SqliteSessionService>,
    pub api_key: Option<String>,
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

#[derive(Debug, Deserialize)]
pub struct UnplugLogQuery {
    pub count: Option<u32>,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct UnplugLogResponse {
    #[serde(rename = "Id")]
    pub id: String,
    #[serde(rename = "Timestamp")]
    pub timestamp: String,
    #[serde(rename = "Station")]
    pub station: String,
    #[serde(rename = "Started")]
    pub started: String,
    #[serde(rename = "Ended")]
    pub ended: String,
    #[serde(rename = "Wh")]
    pub wh: String,
    #[serde(rename = "CardId")]
    pub card_id: String,
}

fn configure_protected_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(
        web::resource("/unplug-log")
            .wrap(from_fn(api_key_middleware))
            .route(web::get().to(get_unplug_log_events_endpoint)),
    )
    .service(
        web::resource("/sessions/carport/latest")
            .wrap(from_fn(api_key_middleware))
            .route(web::get().to(get_carport_latest_report100_endpoint)),
    )
    .service(
        web::resource("/sessions/entrance/latest")
            .wrap(from_fn(api_key_middleware))
            .route(web::get().to(get_entrance_latest_report100_endpoint)),
    );
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(web::resource("/health").route(web::get().to(health)))
        .configure(configure_protected_routes)
        .service(
            web::scope(API_V1_PREFIX)
                .service(web::resource("/health").route(web::get().to(health)))
                .configure(configure_protected_routes),
        );
}

fn unauthorized_response() -> HttpResponse {
    HttpResponse::Unauthorized()
        .insert_header((header::WWW_AUTHENTICATE, "Bearer"))
        .json(serde_json::json!({
            "error": "missing or invalid api key"
        }))
}

fn provided_bearer_token(req: &ServiceRequest) -> Option<&str> {
    req.headers()
        .get(header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
}

async fn api_key_middleware(
    req: ServiceRequest,
    next: Next<impl MessageBody>,
) -> Result<ServiceResponse<EitherBody<impl MessageBody>>, Error> {
    let Some(state) = req.app_data::<web::Data<ApiState>>() else {
        return Ok(next.call(req).await?.map_into_left_body());
    };

    let Some(expected_api_key) = state.api_key.as_deref() else {
        return Ok(next.call(req).await?.map_into_left_body());
    };

    if provided_bearer_token(&req) != Some(expected_api_key) {
        return Ok(req
            .into_response(unauthorized_response())
            .map_into_right_body());
    }

    Ok(next.call(req).await?.map_into_left_body())
}

async fn health() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({ "status": "ok" }))
}

async fn get_carport_latest_report100_endpoint(state: web::Data<ApiState>) -> impl Responder {
    latest_report100_response(&state, "carport")
}

async fn get_entrance_latest_report100_endpoint(state: web::Data<ApiState>) -> impl Responder {
    latest_report100_response(&state, "entrance")
}

async fn get_unplug_log_events_endpoint(
    state: web::Data<ApiState>,
    query: web::Query<UnplugLogQuery>,
) -> impl Responder {
    const DEFAULT_COUNT: u32 = 5;
    const MAX_COUNT: u32 = 500;

    let requested_count = query.count.unwrap_or(DEFAULT_COUNT);
    if requested_count == 0 {
        return HttpResponse::BadRequest().json(serde_json::json!({
            "error": "query parameter 'count' must be >= 1"
        }));
    }
    let clamped_count = requested_count.min(MAX_COUNT);

    let Some(service) = &state.session_query_service else {
        return HttpResponse::ServiceUnavailable().json(serde_json::json!({
            "error": "unplug log query service is not configured"
        }));
    };

    match service.list_recent_unplug_log_events(clamped_count) {
        Ok(entries) => HttpResponse::Ok().json(
            entries
                .into_iter()
                .map(|entry| UnplugLogResponse {
                    id: entry.id,
                    timestamp: entry.timestamp,
                    station: entry.station,
                    started: entry.started,
                    ended: entry.ended,
                    wh: entry.wh,
                    card_id: entry.card_id,
                })
                .collect::<Vec<_>>(),
        ),
        Err(error) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("failed to load unplug log events: {error}")
        })),
    }
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
        "reports 100-130 do not contain started/end timestamps and E Pres >= 0".to_string();
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
    match (view.started, view.ended, view.kwh) {
        (Some(started), Some(ended), Some(kwh)) => started > 0 && ended >= started && kwh >= 0.0,
        _ => false,
    }
}

fn extract_latest_station_session_view(
    object: &serde_json::Map<String, Value>,
    now_ms: i64,
) -> LatestStationSessionView {
    let sec_from_report = find_number_from_object(object, &["Sec", "sec", "Seconds", "seconds"]);
    let kwh = parse_session_kwh_from_object(object);
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
    .and_then(|value| parse_session_timestamp_field(value, now_ms, sec_from_report));
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
    .and_then(|value| parse_session_timestamp_field(value, now_ms, sec_from_report));
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

fn parse_session_kwh_from_object(object: &serde_json::Map<String, Value>) -> Option<f64> {
    if let Some(kwh) =
        find_number_from_object(object, &["Energy Session", "energy_present_session"])
    {
        return Some(kwh);
    }

    find_number_from_object(object, &["E Pres", "E pres"]).map(|value| value / 10.0)
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
        Value::String(text) => parse_number_text(text),
        _ => None,
    }
}

fn parse_number_text(text: &str) -> Option<f64> {
    let trimmed = text.trim();
    if trimmed.is_empty() {
        return None;
    }

    if trimmed.contains(',') && !trimmed.contains('.') {
        let mut parts = trimmed.split(',');
        let head = parts.next()?;
        let tail = parts.next()?;
        let has_more_parts = parts.next().is_some();
        let head_digits = head.trim_start_matches('-');
        let comma_looks_like_thousands_separator = !has_more_parts
            && head_digits.len() >= 3
            && head_digits.chars().all(|char| char.is_ascii_digit())
            && tail.len() == 3
            && tail.chars().all(|char| char.is_ascii_digit());
        if comma_looks_like_thousands_separator {
            return trimmed.replace(',', "").parse::<f64>().ok();
        }
    }

    trimmed.replace(',', ".").parse::<f64>().ok()
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

fn parse_session_timestamp_field(
    value: &Value,
    now_ms: i64,
    sec_from_report100: Option<f64>,
) -> Option<i64> {
    if let Some(raw_numeric) = parse_number(value)
        && raw_numeric <= 0.0
    {
        return None;
    }

    parse_session_timestamp_ms(value, now_ms, sec_from_report100)
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

    use actix_web::{App, body::to_bytes, http::StatusCode, http::header, test, web};
    use chrono::Utc;
    use serde_json::json;

    use super::{ApiState, Report100Station, configure_routes, parse_absolute_timestamp_ms};
    use crate::app::services::{SessionCommandHandler, SqliteSessionService};
    use crate::domain::models::NewUnplugLogRecord;
    use crate::test_support::open_test_connection;

    fn empty_state() -> ApiState {
        ApiState {
            report100_stations: Vec::new(),
            session_query_service: None,
            api_key: None,
        }
    }

    fn state_with_api_key(api_key: &str) -> ApiState {
        ApiState {
            api_key: Some(api_key.to_string()),
            ..empty_state()
        }
    }

    #[actix_web::test]
    async fn parse_session_kwh_from_object_converts_e_pres_thousands_string_to_kwh() {
        let payload = json!({ "E Pres": "285,000" });
        let object = payload
            .as_object()
            .expect("payload fixture should be an object");

        let parsed = super::parse_session_kwh_from_object(object);

        assert_eq!(parsed, Some(28500.0));
    }

    #[actix_web::test]
    async fn parse_session_kwh_from_object_keeps_energy_session_comma_decimal_as_kwh() {
        let payload = json!({ "Energy Session": "20,501" });
        let object = payload
            .as_object()
            .expect("payload fixture should be an object");

        let parsed = super::parse_session_kwh_from_object(object);

        assert_eq!(parsed, Some(20.501));
    }

    #[actix_web::test]
    async fn parse_session_kwh_from_object_converts_small_e_pres_to_kwh() {
        let payload = json!({ "E Pres": 285 });
        let object = payload
            .as_object()
            .expect("payload fixture should be an object");

        let parsed = super::parse_session_kwh_from_object(object);

        assert_eq!(parsed, Some(28.5));
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
    async fn sessions_endpoint_is_reachable_without_api_key() {
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

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn versioned_health_endpoint_returns_ok() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(empty_state()))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get().uri("/api/v1/health").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn versioned_sessions_endpoint_is_reachable_without_api_key() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(empty_state()))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/api/v1/sessions/carport/latest")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn health_endpoint_stays_open_with_configured_api_key() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state_with_api_key("secret-token")))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get().uri("/api/v1/health").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn versioned_sessions_endpoint_rejects_missing_bearer_token_when_api_key_is_configured() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state_with_api_key("secret-token")))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/api/v1/sessions/carport/latest")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(json["error"], "missing or invalid api key");
    }

    #[actix_web::test]
    async fn legacy_sessions_endpoint_rejects_wrong_bearer_token_when_api_key_is_configured() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state_with_api_key("secret-token")))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/carport/latest")
            .insert_header((header::AUTHORIZATION, "Bearer wrong-token"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::UNAUTHORIZED);
    }

    #[actix_web::test]
    async fn versioned_sessions_endpoint_accepts_matching_bearer_token_when_api_key_is_configured()
    {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state_with_api_key("secret-token")))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/api/v1/sessions/carport/latest")
            .insert_header((header::AUTHORIZATION, "Bearer secret-token"))
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn unplug_log_endpoint_returns_latest_entries_limited_by_count() {
        let connection = Arc::new(Mutex::new(open_test_connection("api-unplug-log-list")));
        let service = SqliteSessionService::new(Arc::clone(&connection));

        service
            .insert_unplug_log_event(&NewUnplugLogRecord {
                timestamp: "2026-03-04 09:00".to_string(),
                station: "Carport".to_string(),
                started: "2026-03-04 08:00".to_string(),
                ended: "2026-03-04 09:00".to_string(),
                wh: "3200.0".to_string(),
                card_id: "CARD-1".to_string(),
            })
            .expect("first insert should succeed");
        service
            .insert_unplug_log_event(&NewUnplugLogRecord {
                timestamp: "2026-03-04 10:00".to_string(),
                station: "Entrance".to_string(),
                started: "2026-03-04 09:30".to_string(),
                ended: "2026-03-04 10:00".to_string(),
                wh: "0.0".to_string(),
                card_id: "CARD-2".to_string(),
            })
            .expect("second insert should succeed");
        service
            .insert_unplug_log_event(&NewUnplugLogRecord {
                timestamp: "2026-03-04 11:00".to_string(),
                station: "Carport".to_string(),
                started: "n/a".to_string(),
                ended: "n/a".to_string(),
                wh: "0.0".to_string(),
                card_id: "CARD-3".to_string(),
            })
            .expect("third insert should succeed");

        let mut state = empty_state();
        state.session_query_service = Some(service);
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/unplug-log?count=2")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(json.as_array().map(std::vec::Vec::len), Some(2));
        assert_eq!(json[0]["Timestamp"], "2026-03-04 11:00");
        assert_eq!(json[0]["CardId"], "CARD-3");
        assert_eq!(json[1]["Timestamp"], "2026-03-04 10:00");
        assert_eq!(json[1]["CardId"], "CARD-2");
    }

    #[actix_web::test]
    async fn unplug_log_endpoint_rejects_count_zero() {
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(empty_state()))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/unplug-log?count=0")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::BAD_REQUEST);
        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(json["error"], "query parameter 'count' must be >= 1");
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
            while let Ok((size, from)) = responder.recv_from(&mut buffer) {
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                if cmd == "report 100" {
                    let payload = r#"{"E Pres":123400,"started":"2026-03-01 16:40:19.000","ended":"2026-03-02 04:01:59.000","RFID tag":"ABC123"}"#;
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
        assert_eq!(json["kWh"], 12340.0);
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
    async fn entrance_latest_endpoint_falls_back_to_report101_when_report100_contains_zero_values()
    {
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
            while let Ok((size, from)) = responder.recv_from(&mut buffer) {
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                let payload = match cmd.as_str() {
                    "report 100" => {
                        r#"{"E Pres":71077,"Sec":195395,"started[s]":191012,"ended[s]":0,"started":"191012000","ended":"0","RFID tag":"E100"}"#
                    }
                    "report 101" => {
                        r#"{"E Pres":65432,"Sec":195395,"started[s]":182170,"ended[s]":184901,"RFID tag":"E101"}"#
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
            logical_name: "entrance".to_string(),
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
            .uri("/sessions/entrance/latest")
            .insert_header(("X-API-Key", "1r0m"))
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");

        assert_eq!(json["kWh"], 6543.2);
        assert_eq!(json["reportId"], 101);
        assert_eq!(json["CardId"], "E101");

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
            while let Ok((size, from)) = responder.recv_from(&mut buffer) {
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                let payload = match cmd.as_str() {
                    "report 100" => {
                        r#"{"E Pres":98700,"started":"2026-03-01 16:40:19.000","ended":0,"RFID tag":"ABC123"}"#
                    }
                    "report 101" => {
                        r#"{"E Pres":76500,"started":"2026-03-01 16:40:19.000","ended":"2026-03-02 04:01:59.000","RFID tag":"XYZ999"}"#
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
        assert_eq!(json["kWh"], 7650.0);
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
    async fn carport_latest_endpoint_falls_back_to_report101_when_ended_seconds_is_zero() {
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
            while let Ok((size, from)) = responder.recv_from(&mut buffer) {
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                let payload = match cmd.as_str() {
                    "report 100" => {
                        r#"{"E Pres":71077,"Sec":195395,"started[s]":191012,"ended[s]":0,"started":"191012000","ended":"0","RFID tag":"ABC100"}"#
                    }
                    "report 101" => {
                        r#"{"E Pres":89100,"Sec":195395,"started[s]":182170,"ended[s]":184901,"RFID tag":"ABC101"}"#
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

        assert_eq!(json["kWh"], 8910.0);
        assert_eq!(json["reportId"], 101);
        assert_eq!(json["CardId"], "ABC101");

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
            while let Ok((size, from)) = responder.recv_from(&mut buffer) {
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
                        r#"{"E Pres":42000,"started":"2026-03-01 16:40:19.000","ended":0,"RFID tag":"R100"}"#
                    }
                    "report 101" => {
                        r#"{"E Pres":0,"started":"2026-03-01 16:40:19.000","ended":"2026-03-02 04:01:59.000","RFID tag":"R101"}"#
                    }
                    "report 102" => {
                        r#"{"E Pres":12300,"started":null,"ended":"2026-03-02 04:01:59.000","RFID tag":"R102"}"#
                    }
                    "report 103" => {
                        r#"{"E Pres":65400,"started":"2026-03-01 16:40:19.000","ended":"2026-03-02 04:01:59.000","RFID tag":"R103"}"#
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
        assert_eq!(json["kWh"], 0.0);
        assert_eq!(json["reportId"], 101);
        assert_eq!(json["started"], expected_started);
        assert_eq!(json["ended"], expected_ended);
        assert_eq!(json["CardId"], "R101");

        let commands = seen_commands
            .lock()
            .expect("command log mutex should be lockable")
            .clone();
        assert_eq!(commands, vec!["report 100", "report 101"]);

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
    async fn carport_latest_endpoint_accepts_zero_kwh_when_timestamps_are_complete() {
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
            while let Ok((size, from)) = responder.recv_from(&mut buffer) {
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                let payload = match cmd.as_str() {
                    "report 100" => Some(
                        r#"{"E Pres":0,"started":"2026-03-04 15:41:00.000","ended":"2026-03-05 07:29:00.000","RFID tag":"Z100"}"#,
                    ),
                    "report 101" => Some(
                        r#"{"E Pres":65400,"started":"2026-03-04 15:40:00.000","ended":"2026-03-05 07:30:00.000","RFID tag":"R101"}"#,
                    ),
                    _ => None,
                };
                if let Some(payload) = payload {
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

        assert_eq!(json["kWh"], 0.0);
        assert_eq!(json["reportId"], 100);
        assert_eq!(json["CardId"], "Z100");

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
            while let Ok((size, from)) = responder.recv_from(&mut buffer) {
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                if cmd == "report 100" {
                    let payload = r#"{"E Pres":42000,"Sec":"40000000","started":"26332000","ended":"35131000","RFID tag":"REL1"}"#;
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
        assert_eq!(json["kWh"], 4200.0);
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
    async fn carport_latest_endpoint_uses_sec_fallback_when_absolute_numeric_timestamp_is_implausible()
     {
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
            while let Ok((size, from)) = responder.recv_from(&mut buffer) {
                let cmd = String::from_utf8_lossy(&buffer[..size]).trim().to_string();
                if cmd == "shutdown-test-responder" {
                    break;
                }
                if cmd == "report 100" {
                    let payload = r#"{"E Pres":42000,"Sec":"40000000","started":26332000,"ended":35131000,"RFID tag":"REL2"}"#;
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
        assert_eq!(json["kWh"], 4200.0);
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
            while let Ok((size, from)) = responder.recv_from(&mut buffer) {
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
        assert_eq!(json["kWh"], 8198.4);
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
            while let Ok((size, from)) = responder.recv_from(&mut buffer) {
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
        let expected_ended_min = (now_before as f64 - (sec_now - ended_raw) * 1000.0) as i64 - 2000;
        let expected_ended_max = (now_after as f64 - (sec_now - ended_raw) * 1000.0) as i64 + 2000;

        let started = json["started"].as_i64().expect("started should be i64");
        let ended = json["ended"].as_i64().expect("ended should be i64");
        assert!(started >= expected_started_min && started <= expected_started_max);
        assert!(ended >= expected_ended_min && ended <= expected_ended_max);
        assert_eq!(json["kWh"], 2006.4);
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
            while let Ok((size, from)) = responder.recv_from(&mut buffer) {
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
        assert_eq!(json["kWh"], 8198.4);
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
