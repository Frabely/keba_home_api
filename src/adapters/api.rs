use actix_web::{HttpResponse, Responder, get, web};
use chrono::{SecondsFormat, Utc};
use serde::{Deserialize, Serialize};

use crate::app::services::{ServiceError, SessionQueryHandler, SqliteSessionService};

#[derive(Clone)]
pub struct ApiState {
    pub session_queries: SqliteSessionService,
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

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use actix_web::{App, body::to_bytes, http::StatusCode, test, web};
    use rusqlite::Connection;

    use crate::adapters::db::{
        NewLogEventRecord, NewSessionRecord, insert_log_event, insert_session,
    };
    use crate::app::services::SqliteSessionService;
    use crate::test_support::open_test_connection;

    use super::{ApiState, configure_routes};

    fn build_state_with_migrated_db(name: &str) -> (ApiState, Arc<Mutex<Connection>>) {
        let connection = open_test_connection(name);
        let shared_connection = Arc::new(Mutex::new(connection));

        (
            ApiState {
                session_queries: SqliteSessionService::new(Arc::clone(&shared_connection)),
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
}
