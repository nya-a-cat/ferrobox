mod audit;
mod auth;

use std::{
    collections::{BTreeMap, HashMap},
    sync::Arc,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use audit::AuditLog;
use auth::TokenDigest;
use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use ferrobox_core::{
    ExecRequest, ListDirectoryRequest, NetworkMode, ReadFileRequest, RuntimeError,
    RuntimeErrorKind, SandboxId, SandboxPath, SandboxRuntime, SandboxSpec, SandboxState,
    WriteFileRequest,
};
use serde::{Deserialize, Serialize};
use tokio::sync::RwLock;
use tower_http::{limit::RequestBodyLimitLayer, trace::TraceLayer};

const MAX_REQUEST_BODY: usize = 65 * 1024 * 1024;

#[derive(Clone)]
pub struct AppState {
    inner: Arc<AppStateInner>,
}

struct AppStateInner {
    runtime: Arc<dyn SandboxRuntime>,
    records: RwLock<HashMap<SandboxId, SandboxRecord>>,
    audit: AuditLog,
}

#[derive(Clone)]
struct SandboxRecord {
    token: TokenDigest,
    state: SandboxState,
    node_id: String,
    expires_at: Instant,
    expires_at_unix_ms: u128,
}

impl AppState {
    pub async fn new(
        runtime: Arc<dyn SandboxRuntime>,
        audit_path: impl Into<std::path::PathBuf>,
    ) -> std::io::Result<Self> {
        Ok(Self {
            inner: Arc::new(AppStateInner {
                runtime,
                records: RwLock::new(HashMap::new()),
                audit: AuditLog::open(audit_path).await?,
            }),
        })
    }
}

pub fn router(state: AppState) -> Router {
    Router::new()
        .route("/healthz", get(health))
        .route("/v1/sandboxes", post(create_sandbox))
        .route(
            "/v1/sandboxes/{id}",
            get(get_sandbox).delete(delete_sandbox),
        )
        .route("/v1/sandboxes/{id}/commands", post(execute_command))
        .route("/v1/sandboxes/{id}/files", put(write_file).get(read_file))
        .route("/v1/sandboxes/{id}/directories", get(list_directory))
        .route("/v1/sandboxes/{id}/pause", post(pause_sandbox))
        .route("/v1/sandboxes/{id}/resume", post(resume_sandbox))
        .layer(RequestBodyLimitLayer::new(MAX_REQUEST_BODY))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

async fn health() -> StatusCode {
    StatusCode::NO_CONTENT
}

#[derive(Debug, Deserialize)]
struct CreateSandboxRequest {
    template: String,
    #[serde(default = "default_cpu")]
    cpu_count: u8,
    #[serde(default = "default_memory")]
    memory_mb: u32,
    #[serde(default = "default_ttl")]
    timeout_seconds: u64,
    #[serde(default)]
    network: NetworkRequest,
}

#[derive(Debug, Default, Deserialize)]
struct NetworkRequest {
    #[serde(default)]
    internet_access: bool,
}

#[derive(Debug, Serialize)]
struct CreateSandboxResponse {
    sandbox_id: SandboxId,
    node_id: String,
    state: SandboxState,
    token: String,
    expires_at_unix_ms: u128,
}

async fn create_sandbox(
    State(state): State<AppState>,
    Json(request): Json<CreateSandboxRequest>,
) -> Result<(StatusCode, Json<CreateSandboxResponse>), ApiError> {
    let spec = SandboxSpec {
        template_id: request.template,
        cpu_count: request.cpu_count,
        memory_mb: request.memory_mb,
        timeout_seconds: request.timeout_seconds,
        network: if request.network.internet_access {
            NetworkMode::Internet
        } else {
            NetworkMode::Disabled
        },
    };
    spec.validate()
        .map_err(|error| ApiError::bad_request(error.to_string()))?;
    let details = BTreeMap::from([
        ("template".to_owned(), spec.template_id.clone()),
        ("network".to_owned(), format!("{:?}", spec.network)),
    ]);
    state
        .inner
        .audit
        .record(None, "create", "started", &details)
        .await
        .map_err(ApiError::internal)?;
    let handle = match state.inner.runtime.create(spec.clone()).await {
        Ok(handle) => handle,
        Err(error) => {
            let error_message = error.to_string();
            state
                .inner
                .audit
                .record(
                    None,
                    "create",
                    "failed",
                    &BTreeMap::from([("error".to_owned(), error_message)]),
                )
                .await
                .map_err(ApiError::internal)?;
            return Err(ApiError::from_runtime(error));
        }
    };
    let (token, digest) = TokenDigest::issue();
    let expires_at = Instant::now() + Duration::from_secs(spec.timeout_seconds);
    let expires_at_unix_ms =
        unix_millis().saturating_add(u128::from(spec.timeout_seconds).saturating_mul(1000));
    state.inner.records.write().await.insert(
        handle.sandbox_id.clone(),
        SandboxRecord {
            token: digest,
            state: handle.state,
            node_id: handle.node_id.clone(),
            expires_at,
            expires_at_unix_ms,
        },
    );
    state
        .inner
        .audit
        .record(
            Some(&handle.sandbox_id.to_string()),
            "create",
            "succeeded",
            &details,
        )
        .await
        .map_err(ApiError::internal)?;
    spawn_ttl_reaper(
        state.clone(),
        handle.sandbox_id.clone(),
        spec.timeout_seconds,
    );
    Ok((
        StatusCode::CREATED,
        Json(CreateSandboxResponse {
            sandbox_id: handle.sandbox_id,
            node_id: handle.node_id,
            state: handle.state,
            token,
            expires_at_unix_ms,
        }),
    ))
}

#[derive(Debug, Serialize)]
struct SandboxResponse {
    sandbox_id: SandboxId,
    node_id: String,
    state: SandboxState,
    expires_at_unix_ms: u128,
}

async fn get_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<Json<SandboxResponse>, ApiError> {
    let id = parse_id(&id)?;
    let record = authorize(&state, &id, &headers).await?;
    Ok(Json(SandboxResponse {
        sandbox_id: id,
        node_id: record.node_id,
        state: record.state,
        expires_at_unix_ms: record.expires_at_unix_ms,
    }))
}

#[derive(Debug, Deserialize)]
struct ExecuteCommandRequest {
    argv: Vec<String>,
    #[serde(default = "default_cwd")]
    cwd: String,
    #[serde(default)]
    environment: BTreeMap<String, String>,
    #[serde(default = "default_command_timeout")]
    timeout_seconds: u64,
    #[serde(default = "default_output_limit")]
    max_output_bytes: u64,
}

#[derive(Debug, Serialize)]
struct ExecuteCommandResponse {
    process_id: String,
    termination: ferrobox_core::ExecTermination,
    stdout: String,
    stderr: String,
    stdout_base64: String,
    stderr_base64: String,
    truncation: ferrobox_core::OutputTruncation,
}

async fn execute_command(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<ExecuteCommandRequest>,
) -> Result<Json<ExecuteCommandResponse>, ApiError> {
    let id = parse_id(&id)?;
    authorize_running(&state, &id, &headers).await?;
    let request = ExecRequest {
        argv: request.argv,
        cwd: SandboxPath::new(request.cwd)
            .map_err(|error| ApiError::bad_request(error.to_string()))?,
        environment: request.environment,
        timeout_seconds: request.timeout_seconds,
        max_output_bytes: request.max_output_bytes,
    };
    let result = state
        .inner
        .runtime
        .execute(&id, request)
        .await
        .map_err(ApiError::from_runtime)?;
    let details = BTreeMap::from([("process_id".to_owned(), result.process_id.to_string())]);
    state
        .inner
        .audit
        .record(Some(&id.to_string()), "execute", "succeeded", &details)
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(ExecuteCommandResponse {
        process_id: result.process_id.to_string(),
        termination: result.termination,
        stdout: String::from_utf8_lossy(&result.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&result.stderr).into_owned(),
        stdout_base64: STANDARD.encode(&result.stdout),
        stderr_base64: STANDARD.encode(&result.stderr),
        truncation: result.truncation,
    }))
}

#[derive(Debug, Deserialize)]
struct WriteFileBody {
    path: String,
    content_base64: String,
    #[serde(default)]
    overwrite: bool,
    mode: Option<u32>,
}

#[derive(Debug, Serialize)]
struct WriteFileResponse {
    bytes_written: u64,
}

async fn write_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Json(request): Json<WriteFileBody>,
) -> Result<Json<WriteFileResponse>, ApiError> {
    let id = parse_id(&id)?;
    authorize_running(&state, &id, &headers).await?;
    let data = STANDARD
        .decode(request.content_base64)
        .map_err(|_| ApiError::bad_request("content_base64 is invalid"))?;
    let result = state
        .inner
        .runtime
        .write(
            &id,
            WriteFileRequest {
                path: SandboxPath::new(request.path)
                    .map_err(|error| ApiError::bad_request(error.to_string()))?,
                data,
                overwrite: request.overwrite,
                mode: request.mode,
            },
        )
        .await
        .map_err(ApiError::from_runtime)?;
    state
        .inner
        .audit
        .record(
            Some(&id.to_string()),
            "write_file",
            "succeeded",
            &BTreeMap::from([("bytes".to_owned(), result.bytes_written.to_string())]),
        )
        .await
        .map_err(ApiError::internal)?;
    Ok(Json(WriteFileResponse {
        bytes_written: result.bytes_written,
    }))
}

#[derive(Debug, Deserialize)]
struct ReadFileQuery {
    path: String,
    #[serde(default)]
    offset: u64,
    #[serde(default = "default_file_read_limit")]
    max_bytes: u64,
}

#[derive(Debug, Serialize)]
struct ReadFileResponse {
    content_base64: String,
    bytes: usize,
    eof: bool,
}

async fn read_file(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(request): Query<ReadFileQuery>,
) -> Result<Json<ReadFileResponse>, ApiError> {
    let id = parse_id(&id)?;
    authorize_running(&state, &id, &headers).await?;
    let result = state
        .inner
        .runtime
        .read(
            &id,
            ReadFileRequest {
                path: SandboxPath::new(request.path)
                    .map_err(|error| ApiError::bad_request(error.to_string()))?,
                offset: request.offset,
                max_bytes: request.max_bytes,
            },
        )
        .await
        .map_err(ApiError::from_runtime)?;
    Ok(Json(ReadFileResponse {
        content_base64: STANDARD.encode(&result.data),
        bytes: result.data.len(),
        eof: result.eof,
    }))
}

#[derive(Debug, Deserialize)]
struct ListDirectoryQuery {
    #[serde(default = "default_cwd")]
    path: String,
}

async fn list_directory(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    Query(request): Query<ListDirectoryQuery>,
) -> Result<Json<ferrobox_core::ListDirectoryResult>, ApiError> {
    let id = parse_id(&id)?;
    authorize_running(&state, &id, &headers).await?;
    let result = state
        .inner
        .runtime
        .list(
            &id,
            ListDirectoryRequest {
                path: SandboxPath::new(request.path)
                    .map_err(|error| ApiError::bad_request(error.to_string()))?,
            },
        )
        .await
        .map_err(ApiError::from_runtime)?;
    Ok(Json(result))
}

async fn pause_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let id = parse_id(&id)?;
    authorize_running(&state, &id, &headers).await?;
    transition(&state, &id, SandboxState::Pausing).await?;
    if let Err(error) = state.inner.runtime.pause(&id).await {
        transition(&state, &id, SandboxState::Failed).await?;
        return Err(ApiError::from_runtime(error));
    }
    transition(&state, &id, SandboxState::Paused).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn resume_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let id = parse_id(&id)?;
    let record = authorize(&state, &id, &headers).await?;
    if record.state != SandboxState::Paused {
        return Err(ApiError::conflict("sandbox is not paused"));
    }
    transition(&state, &id, SandboxState::Resuming).await?;
    if let Err(error) = state.inner.runtime.resume(&id).await {
        transition(&state, &id, SandboxState::Failed).await?;
        return Err(ApiError::from_runtime(error));
    }
    transition(&state, &id, SandboxState::Running).await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_sandbox(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
) -> Result<StatusCode, ApiError> {
    let id = parse_id(&id)?;
    authorize(&state, &id, &headers).await?;
    delete_internal(&state, &id, "user").await?;
    Ok(StatusCode::NO_CONTENT)
}

async fn delete_internal(state: &AppState, id: &SandboxId, reason: &str) -> Result<(), ApiError> {
    transition(state, id, SandboxState::Deleting).await?;
    if let Err(error) = state.inner.runtime.delete(id).await {
        transition(state, id, SandboxState::Failed).await?;
        return Err(ApiError::from_runtime(error));
    }
    transition(state, id, SandboxState::Deleted).await?;
    state.inner.records.write().await.remove(id);
    state
        .inner
        .audit
        .record(
            Some(&id.to_string()),
            "delete",
            "succeeded",
            &BTreeMap::from([("reason".to_owned(), reason.to_owned())]),
        )
        .await
        .map_err(ApiError::internal)
}

fn spawn_ttl_reaper(state: AppState, id: SandboxId, ttl_seconds: u64) {
    tokio::spawn(async move {
        tokio::time::sleep(Duration::from_secs(ttl_seconds)).await;
        let expired = state
            .inner
            .records
            .read()
            .await
            .get(&id)
            .is_some_and(|record| record.expires_at <= Instant::now());
        if expired {
            if let Err(error) = delete_internal(&state, &id, "ttl").await {
                tracing::error!(sandbox_id = %id, error = %error.message, "TTL cleanup failed");
            }
        }
    });
}

async fn transition(state: &AppState, id: &SandboxId, next: SandboxState) -> Result<(), ApiError> {
    let mut records = state.inner.records.write().await;
    let record = records
        .get_mut(id)
        .ok_or_else(|| ApiError::not_found("sandbox does not exist"))?;
    if !record.state.can_transition_to(next) {
        return Err(ApiError::conflict(format!(
            "invalid state transition: {:?} -> {next:?}",
            record.state
        )));
    }
    record.state = next;
    Ok(())
}

async fn authorize(
    state: &AppState,
    id: &SandboxId,
    headers: &HeaderMap,
) -> Result<SandboxRecord, ApiError> {
    let token = bearer(headers)?;
    let record = state
        .inner
        .records
        .read()
        .await
        .get(id)
        .cloned()
        .ok_or_else(|| ApiError::not_found("sandbox does not exist"))?;
    if record.expires_at <= Instant::now() {
        return Err(ApiError::new(
            StatusCode::GONE,
            "sandbox_expired",
            "sandbox token has expired",
        ));
    }
    if !record.token.matches(token) {
        return Err(ApiError::unauthorized("invalid bearer token"));
    }
    Ok(record)
}

async fn authorize_running(
    state: &AppState,
    id: &SandboxId,
    headers: &HeaderMap,
) -> Result<SandboxRecord, ApiError> {
    let record = authorize(state, id, headers).await?;
    if record.state != SandboxState::Running {
        return Err(ApiError::conflict("sandbox is not running"));
    }
    Ok(record)
}

fn bearer(headers: &HeaderMap) -> Result<&str, ApiError> {
    headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
        .filter(|value| !value.is_empty())
        .ok_or_else(|| ApiError::unauthorized("missing bearer token"))
}

fn parse_id(value: &str) -> Result<SandboxId, ApiError> {
    value
        .parse()
        .map_err(|_| ApiError::bad_request("sandbox id is invalid"))
}

fn unix_millis() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
}

const fn default_cpu() -> u8 {
    1
}
const fn default_memory() -> u32 {
    512
}
const fn default_ttl() -> u64 {
    300
}
const fn default_command_timeout() -> u64 {
    30
}
const fn default_output_limit() -> u64 {
    1024 * 1024
}
const fn default_file_read_limit() -> u64 {
    64 * 1024 * 1024
}
fn default_cwd() -> String {
    "/home/sandbox".to_owned()
}

#[derive(Debug)]
struct ApiError {
    status: StatusCode,
    code: &'static str,
    message: String,
}

impl ApiError {
    fn new(status: StatusCode, code: &'static str, message: impl Into<String>) -> Self {
        Self {
            status,
            code,
            message: message.into(),
        }
    }
    fn bad_request(message: impl Into<String>) -> Self {
        Self::new(StatusCode::BAD_REQUEST, "invalid_request", message)
    }
    fn unauthorized(message: impl Into<String>) -> Self {
        Self::new(StatusCode::UNAUTHORIZED, "unauthorized", message)
    }
    fn not_found(message: impl Into<String>) -> Self {
        Self::new(StatusCode::NOT_FOUND, "not_found", message)
    }
    fn conflict(message: impl Into<String>) -> Self {
        Self::new(StatusCode::CONFLICT, "conflict", message)
    }
    fn internal(error: impl std::fmt::Display) -> Self {
        tracing::error!(error = %error, "internal API error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "internal",
            "internal server error",
        )
    }
    fn from_runtime(error: RuntimeError) -> Self {
        let (status, code) = match error.kind() {
            RuntimeErrorKind::NotFound => (StatusCode::NOT_FOUND, "not_found"),
            RuntimeErrorKind::Conflict => (StatusCode::CONFLICT, "conflict"),
            RuntimeErrorKind::InvalidInput => (StatusCode::BAD_REQUEST, "invalid_request"),
            RuntimeErrorKind::Unauthorized => (StatusCode::UNAUTHORIZED, "unauthorized"),
            RuntimeErrorKind::Unsupported => (StatusCode::NOT_IMPLEMENTED, "unsupported"),
            RuntimeErrorKind::Timeout => (StatusCode::GATEWAY_TIMEOUT, "timeout"),
            RuntimeErrorKind::ResourceExhausted => {
                (StatusCode::PAYLOAD_TOO_LARGE, "resource_exhausted")
            }
            RuntimeErrorKind::Unavailable => (StatusCode::SERVICE_UNAVAILABLE, "unavailable"),
            RuntimeErrorKind::Internal => {
                tracing::error!(error = %error, "runtime internal error");
                return Self::internal(error);
            }
        };
        Self::new(status, code, error.to_string())
    }
}

impl IntoResponse for ApiError {
    fn into_response(self) -> Response {
        (
            self.status,
            Json(serde_json::json!({
                "error": {
                    "code": self.code,
                    "message": self.message,
                }
            })),
        )
            .into_response()
    }
}
