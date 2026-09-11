//! Localhost-only HTTP adapter for operator intake, launch, and observation.

use std::convert::Infallible;
use std::net::{Ipv4Addr, Ipv6Addr};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use async_stream::stream;
use axum::extract::{Path, Query, State};
use axum::http::{HeaderMap, HeaderValue, StatusCode, Uri, header};
use axum::response::sse::{Event, KeepAlive, Sse};
use axum::response::{IntoResponse, Response};
use axum::routing::{get, post};
use axum::{Json, Router};
use serde::{Deserialize, Serialize};
use workengine_adapters_store::{SqliteObserver, SqliteStore};
use workengine_application::{AppError, SequencedEvent, SystemClock, WorkQuery, create};
use workengine_domain::{Work, WorkEvent, WorkId, WorkStatus};

include!(concat!(env!("OUT_DIR"), "/web_assets.rs"));

const DEFAULT_PAGE_SIZE: usize = 50;
const MAX_PAGE_SIZE: usize = 100;
const CSP: &str = "default-src 'self'; base-uri 'none'; object-src 'none'; frame-ancestors 'none'; connect-src 'self'; style-src 'self'; script-src 'self'";

#[derive(Clone)]
struct ObserverState {
    query: Arc<Mutex<SqliteObserver>>,
    intake: Arc<Mutex<SqliteStore>>,
    version: String,
    control: Arc<dyn StartControl>,
}

/// Foreground local launch supplied by the CLI composition root.
pub trait StartControl: Send + Sync + 'static {
    fn start(&self, id: &WorkId) -> Result<Work, AppError>;
}

/// Run the local API until the process receives an interrupt.
pub fn serve(
    intake: SqliteStore,
    observer: SqliteObserver,
    control: Arc<dyn StartControl>,
    port: u16,
    version: String,
) -> Result<(), AppError> {
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .map_err(AppError::store)?;
    runtime.block_on(async move {
        let state = ObserverState {
            query: Arc::new(Mutex::new(observer)),
            intake: Arc::new(Mutex::new(intake)),
            version,
            control,
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
        .route("/api/v0/overview", get(overview))
        .route("/api/v0/works", get(list_works).post(create_work))
        .route("/api/v0/works/{work_id}", get(show_work))
        .route("/api/v0/works/{work_id}/start", post(start_work))
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
struct OverviewResponse {
    total_works: usize,
    status_counts: StatusCounts,
}

#[derive(Serialize)]
struct StatusCounts {
    ready: usize,
    running: usize,
    succeeded: usize,
    failed: usize,
    parked: usize,
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
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct CreateWorkRequest {
    goal: String,
    worker_profile: Option<String>,
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

async fn overview(State(state): State<ObserverState>) -> Result<Json<OverviewResponse>, ApiError> {
    let works = with_query(&state, |query| query.list())?;
    let mut status_counts = StatusCounts {
        ready: 0,
        running: 0,
        succeeded: 0,
        failed: 0,
        parked: 0,
    };
    for work in &works {
        match work.status() {
            WorkStatus::Ready => status_counts.ready += 1,
            WorkStatus::Running => status_counts.running += 1,
            WorkStatus::Succeeded => status_counts.succeeded += 1,
            WorkStatus::Failed => status_counts.failed += 1,
            WorkStatus::Parked => status_counts.parked += 1,
        }
    }
    Ok(Json(OverviewResponse {
        total_works: works.len(),
        status_counts,
    }))
}

async fn create_work(
    State(state): State<ObserverState>,
    Json(request): Json<CreateWorkRequest>,
) -> Result<(StatusCode, Json<WorkResponse>), ApiError> {
    let work = with_intake(&state, |store| {
        create(
            store,
            &SystemClock,
            request.goal,
            request.worker_profile.unwrap_or_else(|| "stub".to_owned()),
        )
    })
    .map_err(ApiError::from_create)?;
    Ok((StatusCode::CREATED, Json(work_response(&state, &work)?)))
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

async fn start_work(
    State(state): State<ObserverState>,
    Path(work_id): Path<String>,
) -> Result<Json<WorkResponse>, ApiError> {
    let id = WorkId::parse(work_id).map_err(ApiError::bad_request)?;
    let control = state.control.clone();
    let work = tokio::task::spawn_blocking(move || control.start(&id))
        .await
        .map_err(|error| ApiError::internal(format!("start task failed: {error}")))?
        .map_err(ApiError::from_start)?;
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
    let mut cursor = stream_start_cursor(&headers, &params)?;
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

fn stream_start_cursor(headers: &HeaderMap, params: &EventsParams) -> Result<u64, ApiError> {
    let header_cursor = headers
        .get("last-event-id")
        .and_then(|value| value.to_str().ok());
    parse_event_cursor(header_cursor.or(params.after.as_deref()))
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

fn with_intake<T>(
    state: &ObserverState,
    operation: impl FnOnce(&mut SqliteStore) -> Result<T, AppError>,
) -> Result<T, AppError> {
    let mut intake = state
        .intake
        .lock()
        .map_err(|_| AppError::store("operator intake lock poisoned"))?;
    operation(&mut intake)
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

    fn from_create(error: AppError) -> Self {
        match error {
            AppError::Domain(_) => Self::bad_request(error),
            other => Self::from(other),
        }
    }

    fn from_start(error: AppError) -> Self {
        match error {
            AppError::NotFound(_) => Self::not_found(error.to_string()),
            AppError::Conflict(_) | AppError::Domain(_) | AppError::OutcomeSchema(_) => Self {
                status: StatusCode::CONFLICT,
                code: "start_conflict",
                message: error.to_string(),
            },
            other => Self::from(other),
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
    use serde_json::Value;
    use tower::ServiceExt;
    use workengine_adapters_store::SqliteStore;
    use workengine_application::{SystemClock, WorkStore, create};
    use workengine_domain::{Work, WorkAttributes, WorkEvent, WorkId, WorkStatus};

    use super::*;

    struct DisabledStart;

    impl StartControl for DisabledStart {
        fn start(&self, id: &WorkId) -> Result<Work, AppError> {
            Err(AppError::NotFound(id.clone()))
        }
    }

    fn observer_state(directory: &std::path::Path) -> ObserverState {
        ObserverState {
            query: Arc::new(Mutex::new(SqliteObserver::open(directory).unwrap())),
            intake: Arc::new(Mutex::new(SqliteStore::open(directory).unwrap())),
            version: "test".to_owned(),
            control: Arc::new(DisabledStart),
        }
    }

    fn ready_work(id: &str, goal: &str, profile: &str, created_at: u64) -> Work {
        Work::new(
            WorkId::parse(id).unwrap(),
            WorkAttributes::new(goal, profile).unwrap(),
            created_at,
        )
        .unwrap()
    }

    async fn json(response: Response) -> Value {
        let body = to_bytes(response.into_body(), 1024 * 1024).await.unwrap();
        serde_json::from_slice(&body).unwrap()
    }

    #[tokio::test]
    async fn local_api_intake_is_durable_and_observation_hides_workspace_paths() {
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

        let state = observer_state(directory.path());
        let app = router(state.clone());

        let health = app
            .clone()
            .oneshot(Request::get("/api/v0/health").body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);

        let created = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v0/works")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"goal":"make the queue visible","workerProfile":"stub"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(created.status(), StatusCode::CREATED);
        let created = json(created).await;
        let created_id = created["workId"].as_str().unwrap();
        assert_eq!(created["status"], "ready");
        assert_eq!(created["goal"], "make the queue visible");
        assert_eq!(created["workerProfile"], "stub");
        assert_eq!(created["workspaceBound"], false);

        let default_profile = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v0/works")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(r#"{"goal":"use the default profile"}"#))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(default_profile.status(), StatusCode::CREATED);
        let default_profile = json(default_profile).await;
        assert_eq!(default_profile["workerProfile"], "stub");

        let forbidden = app
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri("/api/v0/works")
                    .header(header::CONTENT_TYPE, "application/json")
                    .body(Body::from(
                        r#"{"goal":"cannot set a workspace","workspaceRoot":"/tmp"}"#,
                    ))
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(forbidden.status(), StatusCode::UNPROCESSABLE_ENTITY);

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
        let created_id = WorkId::parse(created_id).unwrap();
        let created_events = query.events(&created_id).unwrap();
        assert_eq!(created_events.len(), 1);
        assert_eq!(created_events[0].kind().as_str(), "created");
        assert_eq!(query.list().unwrap().len(), 3);
        drop(query);

        let second_observer = SqliteObserver::open(directory.path()).unwrap();
        assert_eq!(
            second_observer.get(&created_id).unwrap().unwrap().status(),
            WorkStatus::Ready
        );
    }

    #[tokio::test]
    async fn overview_filters_and_work_cursor_are_stable() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = SqliteStore::open(directory.path()).unwrap();
        for (id, goal, profile, created_at) in [
            ("work-a", "old", "stub", 10),
            ("work-b", "same timestamp one", "writer", 20),
            ("work-c", "same timestamp two", "writer", 20),
            ("work-d", "newest", "stub", 30),
        ] {
            let work = ready_work(id, goal, profile, created_at);
            store.put(&work, WorkEvent::created(&work)).unwrap();
        }
        drop(store);
        let app = router(observer_state(directory.path()));

        let overview = app
            .clone()
            .oneshot(
                Request::get("/api/v0/overview")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let overview = json(overview).await;
        assert_eq!(overview["totalWorks"], 4);
        assert_eq!(overview["statusCounts"]["ready"], 4);
        assert_eq!(overview["statusCounts"]["running"], 0);

        let first = app
            .clone()
            .oneshot(
                Request::get("/api/v0/works?profile=writer&limit=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let first = json(first).await;
        assert_eq!(first["items"][0]["workId"], "work-c");
        assert_eq!(first["nextCursor"], "20:work-c");

        let second = app
            .clone()
            .oneshot(
                Request::get("/api/v0/works?profile=writer&limit=1&cursor=20%3Awork-c")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let second = json(second).await;
        assert_eq!(second["items"][0]["workId"], "work-b");
        assert!(second["nextCursor"].is_null());

        for path in [
            "/api/v0/works?status=unknown",
            "/api/v0/works?limit=0",
            "/api/v0/works?cursor=not-a-cursor",
        ] {
            let response = app
                .clone()
                .oneshot(Request::get(path).body(Body::empty()).unwrap())
                .await
                .unwrap();
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{path}");
        }
    }

    #[tokio::test]
    async fn observers_replay_events_without_mutating_work_or_duplicates() {
        let directory = tempfile::tempdir().unwrap();
        let mut store = SqliteStore::open(directory.path()).unwrap();
        let work = ready_work("work-observed", "observe safely", "stub", 10);
        store.put(&work, WorkEvent::created(&work)).unwrap();
        let mut running = work.clone();
        running.start().unwrap();
        store
            .put(
                &running,
                WorkEvent::started(&running, WorkStatus::Ready, 11),
            )
            .unwrap();
        drop(store);

        let state = observer_state(directory.path());
        let first_observer = router(state.clone());
        let second_observer = router(state.clone());
        for app in [&first_observer, &second_observer] {
            let response = app
                .clone()
                .oneshot(
                    Request::get("/api/v0/works/work-observed")
                        .body(Body::empty())
                        .unwrap(),
                )
                .await
                .unwrap();
            let snapshot = json(response).await;
            assert_eq!(snapshot["status"], "running");
        }

        let first_page = first_observer
            .clone()
            .oneshot(
                Request::get("/api/v0/events?workId=work-observed&limit=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let first_page = json(first_page).await;
        assert_eq!(first_page["items"][0]["cursor"], "1");
        assert_eq!(first_page["nextCursor"], "1");

        let resumed = second_observer
            .clone()
            .oneshot(
                Request::get("/api/v0/events?workId=work-observed&after=1")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let resumed = json(resumed).await;
        assert_eq!(resumed["items"][0]["cursor"], "2");
        assert_eq!(resumed["items"].as_array().unwrap().len(), 1);

        let query = state.query.lock().unwrap();
        assert_eq!(
            query.get(work.id()).unwrap().unwrap().status(),
            WorkStatus::Running
        );
        assert_eq!(query.events(work.id()).unwrap().len(), 2);
    }

    #[tokio::test]
    async fn embedded_ui_uses_spa_fallback_without_masking_api_404s() {
        let directory = tempfile::tempdir().unwrap();
        drop(SqliteStore::open(directory.path()).unwrap());
        let app = router(observer_state(directory.path()));

        let page = app
            .clone()
            .oneshot(
                Request::get("/works/work-from-a-direct-url")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(page.status(), StatusCode::OK);
        assert_eq!(
            page.headers()[header::CONTENT_TYPE],
            "text/html; charset=utf-8"
        );
        assert_eq!(page.headers()[header::CONTENT_SECURITY_POLICY], CSP);

        let missing_api = app
            .oneshot(
                Request::get("/api/v0/not-a-route")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(missing_api.status(), StatusCode::NOT_FOUND);
        assert_eq!(
            missing_api.headers()[header::CONTENT_TYPE],
            "application/json"
        );
    }

    #[test]
    fn stream_cursor_prefers_last_event_id_and_is_exclusive() {
        let params = EventsParams {
            after: Some("1".to_owned()),
            work_id: None,
            limit: None,
        };
        let mut headers = HeaderMap::new();
        headers.insert("last-event-id", HeaderValue::from_static("2"));
        assert!(matches!(stream_start_cursor(&headers, &params), Ok(2)));
        headers.insert("last-event-id", HeaderValue::from_static("bad"));
        assert!(stream_start_cursor(&headers, &params).is_err());
        headers.clear();
        assert!(matches!(stream_start_cursor(&headers, &params), Ok(1)));
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
