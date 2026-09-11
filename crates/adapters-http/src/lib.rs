//! Localhost-only, read-only HTTP observer adapter.

use std::convert::Infallible;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_stream::stream;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::get;
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use workengine_adapters_store::SqliteObserver;
use workengine_application::{AppError, SequencedEvent, WorkQuery};
use workengine_domain::{Work, WorkEvent, WorkId, WorkStatus};

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: usize = 100;
const CSP: &str = "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; connect-src 'self'; style-src 'self'; script-src 'self'";

#[derive(Clone)]
struct ObserverState {
    query: Arc<Mutex<SqliteObserver>>,
    version: String,
}

/// Run the observer until the process receives an interrupt.
pub fn serve(observer: SqliteObserver, port: u16, version: String) -> Result<(), AppError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(AppError::store)?;
    runtime.block_on(async move {
        let state = ObserverState {
            query: Arc::new(Mutex::new(observer)),
            version,
        };
        let app = router(state);
        let ipv4 = tokio::net::TcpListener::bind((Ipv4Addr::LOCALHOST, port))
            .await
            .map_err(AppError::store)?;
        let ipv6 = tokio::net::TcpListener::bind((Ipv6Addr::LOCALHOST, port))
            .await
            .map_err(AppError::store)?;
        tokio::select! {
            result = axum::serve(ipv4, app.clone()) => result.map_err(AppError::store),
            result = axum::serve(ipv6, app) => result.map_err(AppError::store),
        }
    })
}

fn router(state: ObserverState) -> Router {
    Router::new()
        .route("/api/v0/health", get(health))
        .route("/api/v0/works", get(list_works))
        .route("/api/v0/works/{work_id}", get(show_work))
        .route("/api/v0/events", get(list_events))
        .route("/api/v0/events/stream", get(stream_events))
        .fallback(get(static_asset))
        .with_state(state)
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct HealthResponse {
    status: &'static str,
    version: String,
    store: &'static str,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorResponse {
    code: &'static str,
    message: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct WorkResponse {
    work_id: String,
    status: &'static str,
    goal: String,
    worker_profile: String,
    created_at_unix_ms: String,
    outcome_kind: Option<&'static str>,
    workspace_bound: bool,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct EventResponse {
    cursor: String,
    work_id: String,
    kind: &'static str,
    from: Option<&'static str>,
    to: &'static str,
    outcome_kind: Option<&'static str>,
    created_at_unix_ms: String,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct Page<T> {
    items: Vec<T>,
    next_cursor: Option<String>,
}

#[derive(Deserialize)]
struct WorksParams {
    status: Option<String>,
    profile: Option<String>,
    cursor: Option<String>,
    limit: Option<usize>,
}

#[derive(Deserialize)]
struct EventsParams {
    after: Option<String>,
    #[serde(rename = "workId")]
    work_id: Option<String>,
    limit: Option<usize>,
}

async fn health(State(state): State<ObserverState>) -> Result<Json<HealthResponse>, ApiError> {
    with_query(&state, |query| query.list().map(|_| ()))?;
    Ok(Json(HealthResponse {
        status: "ok",
        version: state.version,
        store: "readable",
    }))
}

async fn list_works(
    State(state): State<ObserverState>,
    Query(params): Query<WorksParams>,
) -> Result<Json<Page<WorkResponse>>, ApiError> {
    let status = params.status.as_deref().map(parse_status).transpose()?;
    let limit = page_size(params.limit)?;
    let cursor = params
        .cursor
        .as_deref()
        .map(parse_work_cursor)
        .transpose()?;
    let mut works = with_query(&state, |query| query.list())?;
    works.retain(|work| {
        status.is_none_or(|expected| work.status() == expected)
            && params
                .profile
                .as_deref()
                .is_none_or(|profile| work.attributes().worker_profile() == profile)
    });
    works.sort_by(|left, right| {
        right
            .created_at_unix_ms()
            .cmp(&left.created_at_unix_ms())
            .then_with(|| right.id().as_str().cmp(left.id().as_str()))
    });
    if let Some(cursor) = cursor {
        works.retain(|work| work_is_after_cursor(work, &cursor));
    }
    let has_more = works.len() > limit;
    works.truncate(limit);
    let items = works
        .iter()
        .map(|work| work_response(&state, work))
        .collect::<Result<Vec<_>, _>>()?;
    let next_cursor = has_more.then(|| work_cursor(works.last().expect("non-empty limited page")));
    Ok(Json(Page { items, next_cursor }))
}

async fn show_work(
    State(state): State<ObserverState>,
    Path(work_id): Path<String>,
) -> Result<Json<WorkResponse>, ApiError> {
    let id = WorkId::parse(work_id).map_err(ApiError::bad_request)?;
    let work = with_query(&state, |query| query.get(&id))?
        .ok_or_else(|| ApiError::not_found("Work was not found"))?;
    Ok(Json(work_response(&state, &work)?))
}

async fn list_events(
    State(state): State<ObserverState>,
    Query(params): Query<EventsParams>,
) -> Result<Json<Page<EventResponse>>, ApiError> {
    let after = parse_event_cursor(params.after.as_deref())?;
    let id = params
        .work_id
        .as_deref()
        .map(WorkId::parse)
        .transpose()
        .map_err(ApiError::bad_request)?;
    let limit = page_size(params.limit)?;
    let mut events = with_query(&state, |query| query.events_after(after, id.as_ref()))?;
    let has_more = events.len() > limit;
    events.truncate(limit);
    let next_cursor = has_more.then(|| {
        events
            .last()
            .expect("non-empty limited page")
            .seq
            .to_string()
    });
    Ok(Json(Page {
        items: events.iter().map(event_response).collect(),
        next_cursor,
    }))
}

async fn stream_events(
    State(state): State<ObserverState>,
    headers: HeaderMap,
    Query(params): Query<EventsParams>,
) -> Result<impl IntoResponse, ApiError> {
    let header_cursor = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok());
    let mut cursor = parse_event_cursor(header_cursor.or(params.after.as_deref()))?;
    let work_id = params
        .work_id
        .as_deref()
        .map(WorkId::parse)
        .transpose()
        .map_err(ApiError::bad_request)?;
    let query = state.query.clone();
    let events = stream! {
        loop {
            let batch = match query.lock() {
                Ok(query) => query.events_after(cursor, work_id.as_ref()),
                Err(_) => Err(AppError::store("observer query lock poisoned")),
            };
            match batch {
                Ok(records) if records.is_empty() => yield Ok::<Event, Infallible>(Event::default().comment("keep-alive")),
                Ok(records) => for record in records.into_iter().take(MAX_PAGE_SIZE) {
                    cursor = record.seq;
                    let data = serde_json::to_string(&event_response(&record)).expect("event response is serializable");
                    yield Ok::<Event, Infallible>(Event::default().id(cursor.to_string()).event("workengine.event").data(data));
                },
                Err(error) => {
                    let data = serde_json::to_string(&ErrorResponse { code: "store_unavailable", message: error.to_string() }).expect("error is serializable");
                    yield Ok::<Event, Infallible>(Event::default().event("workengine.error").data(data));
                    break;
                }
            }
            tokio::time::sleep(Duration::from_millis(250)).await;
        }
    };
    Ok(Sse::new(events).keep_alive(
        KeepAlive::new()
            .interval(Duration::from_secs(15))
            .text("keep-alive"),
    ))
}

async fn static_asset(uri: Uri) -> Response {
    let path = uri.path();
    if path.starts_with("/api/") {
        return ApiError::not_found("API route was not found").into_response();
    }
    let asset = WEB_ASSETS
        .iter()
        .find(|(route, _, _)| *route == path)
        .or_else(|| {
            WEB_ASSETS
                .iter()
                .find(|(route, _, _)| *route == "/index.html")
        });
    let Some((route, body, content_type)) = asset else {
        return (
            StatusCode::INTERNAL_SERVER_ERROR,
            "Embedded UI is missing index.html",
        )
            .into_response();
    };
    let mut response = body.to_vec().into_response();
    response
        .headers_mut()
        .insert(header::CONTENT_TYPE, HeaderValue::from_static(content_type));
    response.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(CSP),
    );
    let cache = if *route == "/index.html" {
        "no-cache"
    } else {
        "public, max-age=31536000, immutable"
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response
}

fn with_query<T>(
    state: &ObserverState,
    operation: impl FnOnce(&SqliteObserver) -> Result<T, AppError>,
) -> Result<T, ApiError> {
    let query = state
        .query
        .lock()
        .map_err(|_| ApiError::internal("observer query lock poisoned"))?;
    operation(&query).map_err(ApiError::from)
}

fn work_response(state: &ObserverState, work: &Work) -> Result<WorkResponse, ApiError> {
    let outcome_kind = with_query(state, |query| {
        query
            .events(work.id())
            .map(|events| events.last().and_then(|event| event.outcome_kind()))
    })?;
    Ok(WorkResponse {
        work_id: work.id().as_str().to_owned(),
        status: work.status().as_str(),
        goal: work.attributes().goal().to_owned(),
        worker_profile: work.attributes().worker_profile().to_owned(),
        created_at_unix_ms: work.created_at_unix_ms().to_string(),
        outcome_kind: outcome_kind.map(|kind| kind.as_str()),
        workspace_bound: work.workspace_root().is_some(),
    })
}

fn event_response(record: &SequencedEvent) -> EventResponse {
    let event: &WorkEvent = &record.event;
    EventResponse {
        cursor: record.seq.to_string(),
        work_id: event.work_id().as_str().to_owned(),
        kind: event.kind().as_str(),
        from: event.from().map(WorkStatus::as_str),
        to: event.to().as_str(),
        outcome_kind: event.outcome_kind().map(|kind| kind.as_str()),
        created_at_unix_ms: event.created_at_unix_ms().to_string(),
    }
}

fn page_size(value: Option<usize>) -> Result<usize, ApiError> {
    match value.unwrap_or(DEFAULT_PAGE_SIZE) {
        1..=MAX_PAGE_SIZE => Ok(value.unwrap_or(DEFAULT_PAGE_SIZE)),
        _ => Err(ApiError::bad_request("limit must be between 1 and 100")),
    }
}

fn parse_status(value: &str) -> Result<WorkStatus, ApiError> {
    value.parse().map_err(ApiError::bad_request)
}

fn parse_event_cursor(value: Option<&str>) -> Result<u64, ApiError> {
    value
        .unwrap_or("0")
        .parse()
        .map_err(|_| ApiError::bad_request("after must be an unsigned event cursor"))
}

fn work_cursor(work: &Work) -> String {
    format!("{}:{}", work.created_at_unix_ms(), work.id().as_str())
}

fn parse_work_cursor(value: &str) -> Result<(u64, String), ApiError> {
    let (created_at, work_id) = value
        .split_once(':')
        .ok_or_else(|| ApiError::bad_request("cursor is invalid"))?;
    Ok((
        created_at
            .parse()
            .map_err(|_| ApiError::bad_request("cursor is invalid"))?,
        work_id.to_owned(),
    ))
}

fn work_is_after_cursor(work: &Work, cursor: &(u64, String)) -> bool {
    work.created_at_unix_ms() < cursor.0
        || (work.created_at_unix_ms() == cursor.0 && work.id().as_str() < cursor.1.as_str())
}

struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn bad_request(error: impl std::fmt::Display) -> Self {
        Self {
            status: StatusCode::BAD_REQUEST,
            code: "invalid_request",
            message: error.to_string(),
        }
    }
    fn not_found(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::NOT_FOUND,
            code: "not_found",
            message: message.into(),
        }
    }
    fn internal(message: impl Into<String>) -> Self {
        Self {
            status: StatusCode::INTERNAL_SERVER_ERROR,
            code: "store_unavailable",
            message: message.into(),
        }
    }
}

impl From<AppError> for ApiError {
    fn from(error: AppError) -> Self {
        Self::internal(error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(ErrorResponse {
                code: self.code,
                message: self.message,
            }),
        )
            .into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::body::{Body, to_bytes};
    use axum::http::Request;
    use tower::ServiceExt;
    use workengine_adapters_store::SqliteStore;
    use workengine_application::{SystemClock, WorkStore, create};
    use workengine_domain::{WorkEvent, WorkStatus};

    use super::*;

    #[tokio::test]
    async fn observer_routes_are_read_only_and_hide_workspace_paths() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = SqliteStore::open(directory.path()).unwrap();
        let work = create(&mut store, &SystemClock, "inspect safely", "stub").unwrap();
        let mut running = work.clone();
        running.start().unwrap();
        store
            .put(
                &running,
                WorkEvent::started(&running, WorkStatus::Ready, work.created_at_unix_ms() + 1),
            )
            .unwrap();
        drop(store);

        let state = ObserverState {
            query: Arc::new(Mutex::new(SqliteObserver::open(directory.path()).unwrap())),
            version: "test".to_owned(),
        };
        let app = router(state.clone());

        let health = app
            .clone()
            .oneshot(Request::get("/api/v0/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);

        let works = app
            .clone()
            .oneshot(
                Request::get("/api/v0/works?status=running")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let body = to_bytes(works.into_body(), 1024 * 1024).await.unwrap();
        let body = String::from_utf8(body.to_vec()).unwrap();
        assert!(body.contains("\"status\":\"running\""));
        assert!(!body.contains("workspaceRoot"));

        let missing = app
            .clone()
            .oneshot(
                Request::get("/api/v0/works/not-a-work-id")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing.status(), StatusCode::NOT_FOUND);

        let stream = app
            .oneshot(
                Request::get("/api/v0/events/stream?after=0")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(stream.status(), StatusCode::OK);
        assert_eq!(stream.headers()[header::CONTENT_TYPE], "text/event-stream");

        let query = state.query.lock().unwrap();
        let after = query.get(work.id()).unwrap().unwrap();
        assert_eq!(after.status(), WorkStatus::Running);
        assert_eq!(query.events(work.id()).unwrap().len(), 2);
    }

    #[test]
    fn work_cursor_orders_newest_first() {
        let cursor = (10, "work-b".to_owned());
        let newer = Work::restore(
            WorkId::parse("work-z").unwrap(),
            WorkStatus::Ready,
            workengine_domain::WorkAttributes::new("goal", "stub").unwrap(),
            None,
            11,
        );
        let older = Work::restore(
            WorkId::parse("work-a").unwrap(),
            WorkStatus::Ready,
            workengine_domain::WorkAttributes::new("goal", "stub").unwrap(),
            None,
            10,
        );
        assert!(!work_is_after_cursor(&newer, &cursor));
        assert!(work_is_after_cursor(&older, &cursor));
    }
}
