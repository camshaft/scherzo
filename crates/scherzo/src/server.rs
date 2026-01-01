use crate::{
    config::{Config, verify_password},
    plugin::PluginRegistry,
};
use anyhow::{Context, Result};
use axum::{
    Router,
    body::Body,
    extract::{Path, State},
    http::{Request, StatusCode},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{delete, get, post, put},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    fs,
    path::PathBuf,
    sync::{Arc, RwLock},
};
use tower_http::trace::TraceLayer;
use uuid::Uuid;

/// Shared application state
#[derive(Clone)]
pub struct AppState {
    config: Arc<Config>,
    jobs: Arc<RwLock<JobStore>>,
    plugin_registry: Arc<PluginRegistry>,
}

/// In-memory job store with metadata
pub struct JobStore {
    jobs: HashMap<Uuid, JobMetadata>,
    storage_dir: PathBuf,
}

/// Metadata for a stored job
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JobMetadata {
    pub id: Uuid,
    pub name: String,
    pub original_filename: Option<String>,
    pub size_bytes: u64,
    pub created_at: String,
    pub status: JobStatus,
    /// The original format uploaded (e.g., "gcode" or "wasm")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub original_format: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum JobStatus {
    Uploaded,
    Enqueued,
    Running,
    Completed,
    Failed,
}

/// Response when a job is successfully uploaded
#[derive(Serialize)]
pub struct UploadResponse {
    pub job_id: Uuid,
    pub url: String,
    /// If the job was compiled from a different format (e.g., "gcode")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub compiled_from: Option<String>,
}

/// Request to rename a job
#[derive(Deserialize)]
pub struct RenameRequest {
    pub name: String,
}

/// Response with job time estimate
#[derive(Serialize)]
pub struct EstimateResponse {
    pub estimated_seconds: f64,
    pub estimated_duration: String,
}

/// Response with job preview/toolpath info
#[derive(Serialize)]
pub struct PreviewResponse {
    pub commands_count: usize,
    pub summary: String,
}

impl AppState {
    pub fn new(config: Config, plugin_registry: PluginRegistry) -> Result<Self> {
        let storage_dir = PathBuf::from(&config.jobs.storage_dir);
        fs::create_dir_all(&storage_dir).context("failed to create jobs storage directory")?;

        let jobs = JobStore {
            jobs: HashMap::new(),
            storage_dir,
        };

        Ok(Self {
            config: Arc::new(config),
            jobs: Arc::new(RwLock::new(jobs)),
            plugin_registry: Arc::new(plugin_registry),
        })
    }
}

impl JobStore {
    fn add_job(&mut self, id: Uuid, metadata: JobMetadata) {
        self.jobs.insert(id, metadata);
    }

    fn get_job(&self, id: &Uuid) -> Option<JobMetadata> {
        self.jobs.get(id).cloned()
    }

    fn remove_job(&mut self, id: &Uuid) -> Option<JobMetadata> {
        self.jobs.remove(id)
    }

    fn update_job(&mut self, id: &Uuid, metadata: JobMetadata) {
        self.jobs.insert(*id, metadata);
    }

    fn job_path(&self, id: &Uuid) -> PathBuf {
        self.storage_dir.join(format!("{}.wasm", id))
    }
}

/// Create the main application router
pub fn create_router(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health_check))
        .route("/jobs", post(upload_job))
        .route("/jobs/{id}", get(get_job))
        .route("/jobs/{id}", delete(delete_job))
        .route("/jobs/{id}/rename", put(rename_job))
        .route("/jobs/{id}/estimate", get(estimate_job))
        .route("/jobs/{id}/preview", get(preview_job))
        .route("/jobs/{id}/enqueue", post(enqueue_job))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            auth_middleware,
        ))
        .layer(TraceLayer::new_for_http())
        .with_state(state)
}

/// Health check endpoint (no auth required)
async fn health_check() -> impl IntoResponse {
    (StatusCode::OK, "OK")
}

/// Basic auth middleware
async fn auth_middleware(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, StatusCode> {
    // Skip auth for health check
    if request.uri().path() == "/health" {
        return Ok(next.run(request).await);
    }

    let auth_config = match &state.config.server.auth {
        Some(auth) => auth,
        None => return Ok(next.run(request).await), // No auth configured
    };

    // Extract Authorization header
    let auth_header = request
        .headers()
        .get("Authorization")
        .and_then(|v| v.to_str().ok());

    if let Some(auth) = auth_header
        && let Some(credentials) = auth.strip_prefix("Basic ")
        && let Ok(decoded) = decode_base64(credentials)
        && let Ok(creds_str) = String::from_utf8(decoded)
        && let Some((username, password)) = creds_str.split_once(':')
        && username == auth_config.username
        && verify_password(password, &auth_config.password_hash)
    {
        return Ok(next.run(request).await);
    }

    Err(StatusCode::UNAUTHORIZED)
}

/// Upload a new job
async fn upload_job(
    State(state): State<AppState>,
    headers: axum::http::HeaderMap,
    body: axum::body::Bytes,
) -> Result<impl IntoResponse, AppError> {
    // Check size limit
    if body.len() as u64 > state.config.jobs.max_size_bytes {
        return Err(AppError::PayloadTooLarge);
    }

    // Determine content type from Content-Type header
    let content_type = headers
        .get(axum::http::header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .unwrap_or("application/wasm");

    // Convert to WebAssembly component based on content type
    let (wasm_bytes, original_format) = if content_type.contains("gcode")
        || content_type.contains("text/plain")
        || content_type.contains("text/x-gcode")
    {
        // It's G-code, compile it with plugin schemas
        tracing::info!("Compiling G-code to WebAssembly component");
        let gcode_source =
            String::from_utf8(body.to_vec()).map_err(|_| AppError::InvalidGCode {
                message: "G-code file must be valid UTF-8".to_string(),
            })?;

        // Build compile options with plugin schemas
        let mut options = scherzo_compile::CompileOptions::default();
        let command_handlers = state.plugin_registry.get_command_handlers();

        for (_handler_id, handler) in command_handlers {
            let schema = scherzo_compile::PluginCommandSchema {
                command: handler.command.clone(),
                params: handler
                    .params
                    .iter()
                    .map(|p| scherzo_compile::PluginFieldSchema {
                        name: p.name.clone(),
                        field_type: convert_field_type(&p.field_type),
                        required: p.required,
                        description: p.description.clone(),
                        default_value: p.default_value.clone(),
                    })
                    .collect(),
                description: handler.description.clone(),
            };
            options
                .plugin_schemas
                .insert(handler.command.clone(), schema);
        }

        let compilation = scherzo_compile::compile_gcode_with_options(&gcode_source, options)
            .map_err(|e| AppError::InvalidGCode {
                message: format!("Failed to compile G-code: {}", e),
            })?;

        (compilation.component, "gcode")
    } else {
        // Assume it's already a WebAssembly component
        (body.to_vec(), "wasm")
    };

    // Validate it's a valid WebAssembly component
    // TODO: Validate that all of the requested interfaces are present
    validate_wasm_component(&wasm_bytes)?;

    // Generate job ID
    let job_id = Uuid::new_v4();

    // Store the job file
    let mut jobs = state.jobs.write().unwrap();
    let job_path = jobs.job_path(&job_id);

    fs::write(&job_path, &wasm_bytes)
        .context("failed to write job file")
        .map_err(|e| AppError::Internal(e.to_string()))?;

    // Create metadata
    let metadata = JobMetadata {
        id: job_id,
        name: format!("job-{}", job_id),
        original_filename: None,
        size_bytes: wasm_bytes.len() as u64,
        created_at: chrono::Utc::now().to_rfc3339(),
        status: JobStatus::Uploaded,
        original_format: Some(original_format.to_string()),
    };

    jobs.add_job(job_id, metadata.clone());

    let response = UploadResponse {
        job_id,
        url: format!("/jobs/{}", job_id),
        compiled_from: if original_format == "gcode" {
            Some("gcode".to_string())
        } else {
            None
        },
    };

    Ok((StatusCode::CREATED, axum::Json(response)))
}

/// Get job metadata
async fn get_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let jobs = state.jobs.read().unwrap();
    let metadata = jobs.get_job(&id).ok_or(AppError::NotFound)?;
    Ok(axum::Json(metadata))
}

/// Delete a job
async fn delete_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let mut jobs = state.jobs.write().unwrap();
    let metadata = jobs.remove_job(&id).ok_or(AppError::NotFound)?;

    // Delete the file
    let job_path = jobs.job_path(&id);
    if job_path.exists() {
        fs::remove_file(&job_path)
            .context("failed to delete job file")
            .map_err(|e| AppError::Internal(e.to_string()))?;
    }

    Ok((StatusCode::OK, axum::Json(metadata)))
}

/// Rename a job
async fn rename_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
    axum::Json(request): axum::Json<RenameRequest>,
) -> Result<impl IntoResponse, AppError> {
    let mut jobs = state.jobs.write().unwrap();
    let mut metadata = jobs.get_job(&id).ok_or(AppError::NotFound)?;

    metadata.name = request.name;
    jobs.update_job(&id, metadata.clone());

    Ok(axum::Json(metadata))
}

/// Get estimated time for a job
async fn estimate_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let jobs = state.jobs.read().unwrap();
    let _metadata = jobs.get_job(&id).ok_or(AppError::NotFound)?;

    // TODO: Actually analyze the job and compute real estimates
    // For now, return a placeholder
    let estimated_seconds = 300.0; // 5 minutes placeholder

    let response = EstimateResponse {
        estimated_seconds,
        estimated_duration: format_duration(estimated_seconds),
    };

    Ok(axum::Json(response))
}

/// Get preview/toolpath information for a job
async fn preview_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let jobs = state.jobs.read().unwrap();
    let _metadata = jobs.get_job(&id).ok_or(AppError::NotFound)?;

    // TODO: Actually analyze the job component and extract command info
    // For now, return placeholder data
    let response = PreviewResponse {
        commands_count: 0,
        summary: "Preview not yet implemented".to_string(),
    };

    Ok(axum::Json(response))
}

/// Enqueue a job for execution
async fn enqueue_job(
    State(state): State<AppState>,
    Path(id): Path<Uuid>,
) -> Result<impl IntoResponse, AppError> {
    let mut jobs = state.jobs.write().unwrap();
    let mut metadata = jobs.get_job(&id).ok_or(AppError::NotFound)?;

    // Update status to enqueued
    metadata.status = JobStatus::Enqueued;
    jobs.update_job(&id, metadata.clone());

    // TODO: Actually enqueue the job in a job queue

    Ok(axum::Json(metadata))
}

/// Validate that the bytes represent a valid WebAssembly component
fn validate_wasm_component(bytes: &[u8]) -> Result<(), AppError> {
    // Use wasmparser to validate the component
    // wasmparser automatically detects and validates components
    let mut validator = wasmparser::Validator::new();

    validator
        .validate_all(bytes)
        .context("invalid WebAssembly component")
        .map_err(|e| AppError::InvalidComponent(e.to_string()))?;

    Ok(())
}

/// Format seconds into a human-readable duration
fn format_duration(seconds: f64) -> String {
    let hours = (seconds / 3600.0).floor();
    let minutes = ((seconds % 3600.0) / 60.0).floor();
    let secs = (seconds % 60.0).floor();

    if hours > 0.0 {
        format!("{}h {}m {}s", hours, minutes, secs)
    } else if minutes > 0.0 {
        format!("{}m {}s", minutes, secs)
    } else {
        format!("{}s", secs)
    }
}

/// Application error types
#[derive(Debug)]
pub enum AppError {
    NotFound,
    PayloadTooLarge,
    InvalidComponent(String),
    InvalidGCode { message: String },
    Internal(String),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let (status, message) = match self {
            AppError::NotFound => (StatusCode::NOT_FOUND, "Job not found"),
            AppError::PayloadTooLarge => (StatusCode::PAYLOAD_TOO_LARGE, "Job file too large"),
            AppError::InvalidComponent(ref msg) => {
                return (StatusCode::BAD_REQUEST, msg.clone()).into_response();
            }
            AppError::InvalidGCode { ref message } => {
                return (StatusCode::BAD_REQUEST, message.clone()).into_response();
            }
            AppError::Internal(ref msg) => {
                return (StatusCode::INTERNAL_SERVER_ERROR, msg.clone()).into_response();
            }
        };
        (status, message).into_response()
    }
}

// Need to add base64 and chrono dependencies
use base64::prelude::*;

fn decode_base64(input: &str) -> Result<Vec<u8>, base64::DecodeError> {
    BASE64_STANDARD.decode(input)
}

/// Convert plugin field type to compile field type
fn convert_field_type(field_type: &crate::plugin::FieldType) -> scherzo_compile::PluginFieldType {
    use crate::plugin::FieldType;
    use scherzo_compile::PluginFieldType;

    match field_type {
        FieldType::Int => PluginFieldType::Int,
        FieldType::Float => PluginFieldType::Float,
        FieldType::String => PluginFieldType::String,
        FieldType::Bool => PluginFieldType::Bool,
        FieldType::ListInt => PluginFieldType::ListInt,
        FieldType::ListFloat => PluginFieldType::ListFloat,
        FieldType::ListString => PluginFieldType::ListString,
    }
}
