use actix_web::{HttpResponse, Responder, get, web};
use chrono::{NaiveDateTime, SecondsFormat, TimeZone, Utc};
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::adapters::keba_udp::KebaUdpClient;
use crate::app::services::{ServiceError, SessionQueryHandler, SqliteSessionService};

#[derive(Clone)]
pub struct ApiState {
    pub session_queries: SqliteSessionService,
    pub report100_stations: Vec<Report100Station>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Report100Station {
    pub logical_name: String,
    pub ip: String,
    pub port: u16,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub id: String,
    pub started_at: Option<String>,
    pub finished_at: String,
    pub duration_ms: i64,
    pub kwh: f64,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

#[derive(Debug, Deserialize)]
pub struct DiagnosticsLogQuery {
    pub limit: Option<u32>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsSessionSummary {
    pub id: String,
    pub status: String,
    pub started_reason: String,
    pub finished_reason: String,
    pub started_at: Option<String>,
    pub finished_at: String,
    pub duration_ms: i64,
    pub kwh: f64,
    pub error_count_during_session: i64,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsDbResponse {
    pub schema_version: u32,
    pub sessions_count: i64,
    pub log_events_count: i64,
    pub latest_session: Option<DiagnosticsSessionSummary>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticsLogEventResponse {
    pub id: String,
    pub created_at: String,
    pub level: String,
    pub code: String,
    pub message: String,
    pub source: String,
    pub station_id: Option<String>,
    pub details_json: Option<String>,
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(health)
        .service(get_latest_session_endpoint)
        .service(get_carport_latest_report100_endpoint)
        .service(get_entrance_latest_report100_endpoint)
        .service(get_recent_session_endpoint)
        .service(list_sessions_endpoint)
        .service(get_db_diagnostics_endpoint)
        .service(list_log_events_diagnostics_endpoint);
}

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({ "status": "ok" }))
}

#[get("/sessions/latest")]
async fn get_latest_session_endpoint(state: web::Data<ApiState>) -> impl Responder {
    match state.session_queries.get_latest_session() {
        Ok(Some(session)) => HttpResponse::Ok().json(SessionResponse {
            id: session.id,
            started_at: session.started_at,
            finished_at: session.finished_at,
            duration_ms: session.duration_ms,
            kwh: session.energy_kwh,
        }),
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "no sessions available"
        })),
        Err(error) => service_error_response(error),
    }
}

#[derive(Debug, Serialize, PartialEq)]
pub struct LatestStationSessionResponse {
    #[serde(rename = "kWh")]
    pub kwh: f64,
    pub started: Option<i64>,
    pub ended: Option<i64>,
    #[serde(rename = "CardId")]
    pub card_id: Value,
}

#[get("/sessions/carport/latest")]
async fn get_carport_latest_report100_endpoint(state: web::Data<ApiState>) -> impl Responder {
    latest_report100_response(&state, "carport")
}

#[get("/sessions/entrance/latest")]
async fn get_entrance_latest_report100_endpoint(state: web::Data<ApiState>) -> impl Responder {
    latest_report100_response(&state, "entrance")
}

#[get("/sessions")]
async fn list_sessions_endpoint(
    state: web::Data<ApiState>,
    query: web::Query<ListQuery>,
) -> impl Responder {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let offset = query.offset.unwrap_or(0);

    match state.session_queries.list_sessions(limit, offset) {
        Ok(sessions) => {
            let mapped: Vec<SessionResponse> = sessions
                .into_iter()
                .map(|session| SessionResponse {
                    id: session.id,
                    started_at: session.started_at,
                    finished_at: session.finished_at,
                    duration_ms: session.duration_ms,
                    kwh: session.energy_kwh,
                })
                .collect();

            HttpResponse::Ok().json(mapped)
        }
        Err(error) => service_error_response(error),
    }
}

#[get("/sessions/recent")]
async fn get_recent_session_endpoint(state: web::Data<ApiState>) -> impl Responder {
    let threshold =
        (Utc::now() - chrono::Duration::minutes(5)).to_rfc3339_opts(SecondsFormat::Millis, true);

    match state.session_queries.get_latest_session_since(&threshold) {
        Ok(Some(session)) => HttpResponse::Ok().json(SessionResponse {
            id: session.id,
            started_at: session.started_at,
            finished_at: session.finished_at,
            duration_ms: session.duration_ms,
            kwh: session.energy_kwh,
        }),
        Ok(None) => HttpResponse::NoContent().finish(),
        Err(error) => service_error_response(error),
    }
}

#[get("/diagnostics/db")]
async fn get_db_diagnostics_endpoint(state: web::Data<ApiState>) -> impl Responder {
    let schema_version = match state.session_queries.get_schema_version() {
        Ok(value) => value,
        Err(error) => return service_error_response(error),
    };
    let sessions_count = match state.session_queries.count_sessions() {
        Ok(value) => value,
        Err(error) => return service_error_response(error),
    };
    let log_events_count = match state.session_queries.count_log_events() {
        Ok(value) => value,
        Err(error) => return service_error_response(error),
    };
    let latest_session = match state.session_queries.get_latest_session() {
        Ok(value) => value,
        Err(error) => return service_error_response(error),
    };

    let latest_session = latest_session.map(|session| DiagnosticsSessionSummary {
        id: session.id,
        status: session.status,
        started_reason: session.started_reason,
        finished_reason: session.finished_reason,
        started_at: session.started_at,
        finished_at: session.finished_at,
        duration_ms: session.duration_ms,
        kwh: session.energy_kwh,
        error_count_during_session: session.error_count_during_session,
    });

    HttpResponse::Ok().json(DiagnosticsDbResponse {
        schema_version,
        sessions_count,
        log_events_count,
        latest_session,
    })
}

#[get("/diagnostics/log-events")]
async fn list_log_events_diagnostics_endpoint(
    state: web::Data<ApiState>,
    query: web::Query<DiagnosticsLogQuery>,
) -> impl Responder {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    match state.session_queries.list_recent_log_events(limit) {
        Ok(events) => {
            let mapped: Vec<DiagnosticsLogEventResponse> = events
                .into_iter()
                .map(|event| DiagnosticsLogEventResponse {
                    id: event.id,
                    created_at: event.created_at,
                    level: event.level,
                    code: event.code,
                    message: event.message,
                    source: event.source,
                    station_id: event.station_id,
                    details_json: event.details_json,
                })
                .collect();
            HttpResponse::Ok().json(mapped)
        }
        Err(error) => service_error_response(error),
    }
}

fn service_error_response(error: ServiceError) -> HttpResponse {
    match error {
        ServiceError::DbLockPoisoned => {
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "database lock poisoned"
            }))
        }
        ServiceError::Database(error) => {
            HttpResponse::InternalServerError().json(serde_json::json!({
                "error": format!("database query failed: {error}")
            }))
        }
    }
}

fn latest_report100_response(state: &ApiState, station_name: &str) -> HttpResponse {
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

    let report100 = match client.get_report100() {
        Ok(payload) => payload,
        Err(error) => {
            return HttpResponse::BadGateway().json(serde_json::json!({
                "error": format!("failed to fetch report 100: {error}")
            }));
        }
    };

    let object = match report100.as_object() {
        Some(object) => object,
        None => {
            return HttpResponse::BadGateway().json(serde_json::json!({
                "error": "report 100 payload must be a json object"
            }));
        }
    };

    let now_ms = Utc::now().timestamp_millis();
    let ended_is_zero_in_report100 = find_value_from_object(
        object,
        &["ended", "Ended", "end", "session_end", "Session End"],
    )
    .and_then(parse_number)
    .map(|value| value == 0.0)
    .unwrap_or(false);

    let report100_view = extract_latest_station_session_view(object, now_ms);
    let effective_view = if ended_is_zero_in_report100 {
        let report101 = match client.get_report101() {
            Ok(payload) => payload,
            Err(error) => {
                return HttpResponse::BadGateway().json(serde_json::json!({
                    "error": format!("failed to fetch report 101: {error}")
                }));
            }
        };
        let object101 = match report101.as_object() {
            Some(object) => object,
            None => {
                return HttpResponse::BadGateway().json(serde_json::json!({
                    "error": "report 101 payload must be a json object"
                }));
            }
        };
        extract_latest_station_session_view(object101, now_ms)
    } else {
        report100_view
    };

    if effective_view.started.is_none()
        || effective_view.ended.is_none()
        || effective_view.ended.unwrap_or(0) <= 0
    {
        return HttpResponse::BadGateway().json(serde_json::json!({
            "error": "report 100/101 payload does not contain valid started/ended timestamps"
        }));
    }
    if effective_view.kwh.is_none() {
        return HttpResponse::BadGateway().json(serde_json::json!({
            "error": "report 100/101 payload does not contain a numeric E Pres value"
        }));
    }

    HttpResponse::Ok().json(LatestStationSessionResponse {
        kwh: effective_view.kwh.unwrap_or(0.0),
        started: effective_view.started,
        ended: effective_view.ended,
        card_id: effective_view.card_id,
    })
}

struct LatestStationSessionView {
    kwh: Option<f64>,
    started: Option<i64>,
    ended: Option<i64>,
    card_id: Value,
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
    );
    let started = find_value_from_object(
        object,
        &["started", "Started", "start", "session_start", "Session Start"],
    )
    .and_then(|value| parse_session_timestamp_ms(value, now_ms, sec_from_report));
    let ended = find_value_from_object(
        object,
        &["ended", "Ended", "end", "session_end", "Session End"],
    )
    .and_then(|value| parse_session_timestamp_ms(value, now_ms, sec_from_report));
    let card_id = find_value_from_object(
        object,
        &[
            "RFID",
            "RFID tag",
            "RFID Tag",
            "CardId",
            "Card ID",
            "card_id",
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

fn find_value_from_object<'a>(
    object: &'a serde_json::Map<String, Value>,
    aliases: &[&str],
) -> Option<&'a Value> {
    for alias in aliases {
        if let Some(value) = object.get(*alias) {
            return Some(value);
        }
    }
    let normalized_aliases: Vec<String> = aliases.iter().map(|alias| normalize_key(alias)).collect();
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

fn find_number_from_object(object: &serde_json::Map<String, Value>, aliases: &[&str]) -> Option<f64> {
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
    match value {
        Value::Number(number) => number.as_i64(),
        Value::String(text) => {
            if let Ok(parsed) = chrono::DateTime::parse_from_rfc3339(text) {
                return Some(parsed.timestamp_millis());
            }
            if let Ok(parsed) = NaiveDateTime::parse_from_str(text, "%Y-%m-%d %H:%M:%S%.3f") {
                return Some(Utc.from_utc_datetime(&parsed).timestamp_millis());
            }
            None
        }
        _ => None,
    }
}

fn parse_session_timestamp_ms(value: &Value, now_ms: i64, sec_from_report100: Option<f64>) -> Option<i64> {
    if let Some(sec_now) = sec_from_report100
        && let Some(raw_seconds) = parse_number(value)
        && (0.0..1_000_000_000_000.0).contains(&raw_seconds)
    {
        let ts = (now_ms as f64) - ((sec_now - raw_seconds) * 1000.0);
        return Some(ts.round() as i64);
    }

    parse_absolute_timestamp_ms(value)
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
    use rusqlite::Connection;

    use crate::adapters::db::{
        NewLogEventRecord, NewSessionRecord, insert_log_event, insert_session,
    };
    use crate::app::services::SqliteSessionService;
    use crate::test_support::open_test_connection;

    use super::{ApiState, Report100Station, configure_routes, parse_absolute_timestamp_ms};

    fn build_state_with_migrated_db(name: &str) -> (ApiState, Arc<Mutex<Connection>>) {
        let connection = open_test_connection(name);
        let shared_connection = Arc::new(Mutex::new(connection));

        (
            ApiState {
                session_queries: SqliteSessionService::new(Arc::clone(&shared_connection)),
                report100_stations: Vec::new(),
            },
            shared_connection,
        )
    }

    fn sample_new_session(
        started_at: Option<&str>,
        finished_at: &str,
        created_at: &str,
        energy_kwh: f64,
    ) -> NewSessionRecord {
        let started_ms = started_at.map(|value| {
            chrono::DateTime::parse_from_rfc3339(value)
                .expect("started_at should parse")
                .timestamp_millis()
        });
        let finished_ms = chrono::DateTime::parse_from_rfc3339(finished_at)
            .expect("finished_at should parse")
            .timestamp_millis();

        NewSessionRecord {
            started_at: started_at.map(ToString::to_string),
            finished_at: finished_at.to_string(),
            duration_ms: started_ms.map_or(0, |value| (finished_ms - value).max(0)),
            energy_kwh,
            source: "debug_file".to_string(),
            status: "completed".to_string(),
            started_reason: "plug_state_transition".to_string(),
            finished_reason: "plug_state_transition".to_string(),
            poll_interval_ms: 1000,
            debounce_samples: 2,
            error_count_during_session: 0,
            station_id: Some("station-a".to_string()),
            created_at: created_at.to_string(),
            raw_report2_start: None,
            raw_report3_start: None,
            raw_report2_end: None,
            raw_report3_end: None,
        }
    }

    #[actix_web::test]
    async fn health_endpoint_returns_ok() {
        let (state, _) = build_state_with_migrated_db("health.sqlite");
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get().uri("/health").to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);
    }

    #[actix_web::test]
    async fn latest_session_returns_404_when_empty() {
        let (state, _) = build_state_with_migrated_db("latest-empty-api.sqlite");
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/latest")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::NOT_FOUND);
    }

    #[actix_web::test]
    async fn latest_session_returns_most_recent_record() {
        let (state, connection) = build_state_with_migrated_db("latest-record-api.sqlite");

        {
            let db = connection.lock().expect("lock should be available");
            insert_session(
                &db,
                &sample_new_session(
                    Some("2026-02-20T10:00:00.000Z"),
                    "2026-02-20T11:00:00.000Z",
                    "2026-02-20T11:00:00.000Z",
                    5.0,
                ),
            )
            .expect("insert should succeed");
            insert_session(
                &db,
                &sample_new_session(
                    Some("2026-02-21T10:00:00.000Z"),
                    "2026-02-21T11:00:00.000Z",
                    "2026-02-21T11:00:00.000Z",
                    6.0,
                ),
            )
            .expect("insert should succeed");
        }

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/latest")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");

        assert_eq!(json["kwh"], 6.0);
    }

    #[actix_web::test]
    async fn latest_session_returns_null_started_at_when_unknown() {
        let (state, connection) = build_state_with_migrated_db("latest-null-started-at-api.sqlite");

        {
            let db = connection.lock().expect("lock should be available");
            insert_session(
                &db,
                &sample_new_session(
                    None,
                    "2026-02-21T11:00:00.000Z",
                    "2026-02-21T11:00:00.000Z",
                    6.0,
                ),
            )
            .expect("insert should succeed");
        }

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/latest")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(json["startedAt"], serde_json::Value::Null);
    }

    #[actix_web::test]
    async fn list_sessions_supports_limit_and_offset() {
        let (state, connection) = build_state_with_migrated_db("list-api.sqlite");

        {
            let db = connection.lock().expect("lock should be available");
            for idx in 0..3 {
                let day = 20 + idx;
                let created_at = format!("2026-02-{day:02}T11:00:00.000Z");
                insert_session(
                    &db,
                    &sample_new_session(
                        Some(&format!("2026-02-{day:02}T10:00:00.000Z")),
                        &created_at,
                        &created_at,
                        5.0 + idx as f64,
                    ),
                )
                .expect("insert should succeed");
            }
        }

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions?limit=2&offset=1")
            .to_request();
        let resp = test::call_service(&app, req).await;

        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        let items = json.as_array().expect("response should be an array");

        assert_eq!(items.len(), 2);
        assert_eq!(items[0]["kwh"], 6.0);
        assert_eq!(items[1]["kwh"], 5.0);
    }

    #[actix_web::test]
    async fn recent_session_returns_no_content_when_none_in_last_five_minutes() {
        let (state, _) = build_state_with_migrated_db("recent-empty-api.sqlite");
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/recent")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::NO_CONTENT);
    }

    #[actix_web::test]
    async fn recent_session_returns_latest_within_last_five_minutes() {
        let (state, connection) = build_state_with_migrated_db("recent-found-api.sqlite");

        {
            let db = connection.lock().expect("lock should be available");
            insert_session(
                &db,
                &sample_new_session(
                    Some("2026-02-20T10:00:00.000Z"),
                    "2026-02-20T10:10:00.000Z",
                    "3026-02-20T10:10:00.000Z",
                    4.5,
                ),
            )
            .expect("insert should succeed");
        }

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;

        let req = test::TestRequest::get()
            .uri("/sessions/recent")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(json["kwh"], 4.5);
    }

    #[actix_web::test]
    async fn diagnostics_db_returns_schema_counts_and_latest_session() {
        let (state, connection) = build_state_with_migrated_db("diagnostics-db-api.sqlite");

        {
            let db = connection.lock().expect("lock should be available");
            insert_session(
                &db,
                &sample_new_session(
                    Some("2026-02-22T10:00:00.000Z"),
                    "2026-02-22T11:00:00.000Z",
                    "2026-02-22T11:00:00.000Z",
                    7.0,
                ),
            )
            .expect("insert should succeed");
            insert_log_event(
                &db,
                &NewLogEventRecord {
                    created_at: "2026-02-22T10:30:00.000Z".to_string(),
                    level: "warn".to_string(),
                    code: "poll.fetch_report2".to_string(),
                    message: "timeout".to_string(),
                    source: "debug_file".to_string(),
                    station_id: None,
                    details_json: Some("{\"x\":1}".to_string()),
                },
            )
            .expect("log insert should succeed");
        }

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;
        let req = test::TestRequest::get().uri("/diagnostics/db").to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        assert_eq!(json["schemaVersion"], 5);
        assert_eq!(json["sessionsCount"], 1);
        assert_eq!(json["logEventsCount"], 1);
        assert_eq!(json["latestSession"]["status"], "completed");
    }

    #[actix_web::test]
    async fn diagnostics_log_events_returns_recent_events() {
        let (state, connection) = build_state_with_migrated_db("diagnostics-logs-api.sqlite");

        {
            let db = connection.lock().expect("lock should be available");
            insert_log_event(
                &db,
                &NewLogEventRecord {
                    created_at: "2026-02-22T10:30:00.000Z".to_string(),
                    level: "warn".to_string(),
                    code: "poll.fetch_report2".to_string(),
                    message: "timeout".to_string(),
                    source: "debug_file".to_string(),
                    station_id: Some("station-a".to_string()),
                    details_json: Some("{\"attempt\":1}".to_string()),
                },
            )
            .expect("log insert should succeed");
            insert_log_event(
                &db,
                &NewLogEventRecord {
                    created_at: "2026-02-22T10:31:00.000Z".to_string(),
                    level: "warn".to_string(),
                    code: "poll.parse_report2".to_string(),
                    message: "invalid payload".to_string(),
                    source: "debug_file".to_string(),
                    station_id: Some("station-a".to_string()),
                    details_json: Some("{\"attempt\":2}".to_string()),
                },
            )
            .expect("log insert should succeed");
        }

        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;
        let req = test::TestRequest::get()
            .uri("/diagnostics/log-events?limit=1")
            .to_request();
        let resp = test::call_service(&app, req).await;
        assert_eq!(resp.status(), StatusCode::OK);

        let body = to_bytes(resp.into_body())
            .await
            .expect("body should be readable");
        let json: serde_json::Value = serde_json::from_slice(&body).expect("body should be json");
        let items = json.as_array().expect("response should be array");
        assert_eq!(items.len(), 1);
        assert_eq!(items[0]["code"], "poll.parse_report2");
        assert_eq!(items[0]["detailsJson"], "{\"attempt\":2}");
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

        let (mut state, _) = build_state_with_migrated_db("report100-carport-api.sqlite");
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
        let expected_ended = parse_absolute_timestamp_ms(&serde_json::json!("2026-03-02 04:01:59.000"))
            .expect("timestamp should parse");
        assert_eq!(json["kWh"], 12.34);
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
        let (state, _) = build_state_with_migrated_db("report100-entrance-missing-api.sqlite");
        let app = test::init_service(
            App::new()
                .app_data(web::Data::new(state))
                .configure(configure_routes),
        )
        .await;
        let req = test::TestRequest::get()
            .uri("/sessions/entrance/latest")
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

        let (mut state, _) = build_state_with_migrated_db("report101-fallback-api.sqlite");
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
        let expected_ended = parse_absolute_timestamp_ms(&serde_json::json!("2026-03-02 04:01:59.000"))
            .expect("timestamp should parse");
        assert_eq!(json["kWh"], 7.65);
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
                    let payload =
                        r#"{"E Pres":4.2,"Sec":"40000000","started":"26332000","ended":"35131000","RFID tag":"REL1"}"#;
                    responder
                        .send_to(payload.as_bytes(), from)
                        .expect("responder send should succeed");
                }
            }
        });

        let (mut state, _) = build_state_with_migrated_db("report100-relative-sec-api.sqlite");
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
        let expected_started_min = (now_before as f64 - (sec_now - started_raw) * 1000.0) as i64 - 2000;
        let expected_started_max = (now_after as f64 - (sec_now - started_raw) * 1000.0) as i64 + 2000;
        let expected_ended_min = (now_before as f64 - (sec_now - ended_raw) * 1000.0) as i64 - 2000;
        let expected_ended_max = (now_after as f64 - (sec_now - ended_raw) * 1000.0) as i64 + 2000;

        let started = json["started"].as_i64().expect("started should be i64");
        let ended = json["ended"].as_i64().expect("ended should be i64");
        assert!(started >= expected_started_min && started <= expected_started_max);
        assert!(ended >= expected_ended_min && ended <= expected_ended_max);
        assert_eq!(json["kWh"], 4.2);
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
}
