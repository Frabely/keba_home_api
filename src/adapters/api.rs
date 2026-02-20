use std::sync::{Arc, Mutex};

use actix_web::{HttpResponse, Responder, get, web};
use rusqlite::Connection;
use serde::{Deserialize, Serialize};

use crate::adapters::db::{get_latest_session, list_sessions};

#[derive(Clone)]
pub struct ApiState {
    pub connection: Arc<Mutex<Connection>>,
}

#[derive(Debug, Serialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct SessionResponse {
    pub id: i64,
    pub plugged_at: String,
    pub unplugged_at: String,
    pub kwh: f64,
}

#[derive(Debug, Deserialize)]
pub struct ListQuery {
    pub limit: Option<u32>,
    pub offset: Option<u32>,
}

pub fn configure_routes(cfg: &mut web::ServiceConfig) {
    cfg.service(health)
        .service(get_latest_session_endpoint)
        .service(list_sessions_endpoint);
}

#[get("/health")]
async fn health() -> impl Responder {
    HttpResponse::Ok().json(serde_json::json!({ "status": "ok" }))
}

#[get("/sessions/latest")]
async fn get_latest_session_endpoint(state: web::Data<ApiState>) -> impl Responder {
    let connection = match state.connection.lock() {
        Ok(connection) => connection,
        Err(_) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "database lock poisoned"
            }));
        }
    };

    match get_latest_session(&connection) {
        Ok(Some(session)) => HttpResponse::Ok().json(SessionResponse {
            id: session.id,
            plugged_at: session.plugged_at,
            unplugged_at: session.unplugged_at,
            kwh: session.kwh,
        }),
        Ok(None) => HttpResponse::NotFound().json(serde_json::json!({
            "error": "no sessions available"
        })),
        Err(error) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("database query failed: {error}")
        })),
    }
}

#[get("/sessions")]
async fn list_sessions_endpoint(
    state: web::Data<ApiState>,
    query: web::Query<ListQuery>,
) -> impl Responder {
    let limit = query.limit.unwrap_or(50).clamp(1, 500);
    let offset = query.offset.unwrap_or(0);

    let connection = match state.connection.lock() {
        Ok(connection) => connection,
        Err(_) => {
            return HttpResponse::InternalServerError().json(serde_json::json!({
                "error": "database lock poisoned"
            }));
        }
    };

    match list_sessions(&connection, limit, offset) {
        Ok(sessions) => {
            let mapped: Vec<SessionResponse> = sessions
                .into_iter()
                .map(|session| SessionResponse {
                    id: session.id,
                    plugged_at: session.plugged_at,
                    unplugged_at: session.unplugged_at,
                    kwh: session.kwh,
                })
                .collect();

            HttpResponse::Ok().json(mapped)
        }
        Err(error) => HttpResponse::InternalServerError().json(serde_json::json!({
            "error": format!("database query failed: {error}")
        })),
    }
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Mutex};

    use actix_web::{App, body::to_bytes, http::StatusCode, test, web};

    use crate::adapters::db::{
        NewSessionRecord, insert_session, open_connection, run_migrations, schema_version,
    };

    use super::{ApiState, configure_routes};

    fn temp_db_path(name: &str) -> std::path::PathBuf {
        let dir = tempfile::tempdir().expect("tempdir should be created");
        let path = dir.path().join(name);
        std::mem::forget(dir);
        path
    }

    fn build_state_with_migrated_db(name: &str) -> ApiState {
        let db_path = temp_db_path(name);
        let mut connection =
            open_connection(db_path.to_string_lossy().as_ref()).expect("db should open");
        run_migrations(&mut connection).expect("migrations should succeed");
        let _ = schema_version(&connection).expect("schema version should be readable");

        ApiState {
            connection: Arc::new(Mutex::new(connection)),
        }
    }

    #[actix_web::test]
    async fn health_endpoint_returns_ok() {
        let state = build_state_with_migrated_db("health.sqlite");
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
        let state = build_state_with_migrated_db("latest-empty-api.sqlite");
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
        let state = build_state_with_migrated_db("latest-record-api.sqlite");

        {
            let connection = state.connection.lock().expect("lock should be available");
            insert_session(
                &connection,
                &NewSessionRecord {
                    plugged_at: "2026-02-20T10:00:00.000Z".to_string(),
                    unplugged_at: "2026-02-20T11:00:00.000Z".to_string(),
                    kwh: 5.0,
                    created_at: "2026-02-20T11:00:00.000Z".to_string(),
                    raw_report2: None,
                    raw_report3: None,
                },
            )
            .expect("insert should succeed");
            insert_session(
                &connection,
                &NewSessionRecord {
                    plugged_at: "2026-02-21T10:00:00.000Z".to_string(),
                    unplugged_at: "2026-02-21T11:00:00.000Z".to_string(),
                    kwh: 6.0,
                    created_at: "2026-02-21T11:00:00.000Z".to_string(),
                    raw_report2: None,
                    raw_report3: None,
                },
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
    async fn list_sessions_supports_limit_and_offset() {
        let state = build_state_with_migrated_db("list-api.sqlite");

        {
            let connection = state.connection.lock().expect("lock should be available");
            for idx in 0..3 {
                let day = 20 + idx;
                let created_at = format!("2026-02-{day:02}T11:00:00.000Z");
                insert_session(
                    &connection,
                    &NewSessionRecord {
                        plugged_at: format!("2026-02-{day:02}T10:00:00.000Z"),
                        unplugged_at: created_at.clone(),
                        kwh: 5.0 + idx as f64,
                        created_at,
                        raw_report2: None,
                        raw_report3: None,
                    },
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
}
